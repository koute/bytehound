//! The compression algorithm.
//!
//! We make use of hash tables to find duplicates. This gives a reasonable compression ratio with a
//! high performance. It has fixed memory usage, which contrary to other approachs, makes it less
//! memory hungry.

use byteorder::{NativeEndian, ByteOrder};

/// Duplication dictionary size.
///
/// Every four bytes is assigned an entry. When this number is lower, fewer entries exists, and
/// thus collisions are more likely, hurting the compression ratio.
const DICTIONARY_SIZE: usize = 4096;

/// A LZ4 block.
///
/// This defines a single compression "unit", consisting of two parts, a number of raw literals,
/// and possibly a pointer to the already encoded buffer from which to copy.
#[derive(Debug)]
struct Block {
    /// The length (in bytes) of the literals section.
    lit_len: usize,
    /// The duplicates section if any.
    ///
    /// Only the last block in a stream can lack of the duplicates section.
    dup: Option<Duplicate>,
}

/// A consecutive sequence of bytes found in already encoded part of the input.
#[derive(Copy, Clone, Debug)]
struct Duplicate {
    /// The number of bytes before our cursor, where the duplicate starts.
    offset: u16,
    /// The length beyond the four first bytes.
    ///
    /// Adding four to this number yields the actual length.
    extra_bytes: usize,
}

/// An LZ4 encoder.
struct Encoder<'a> {
    /// The raw uncompressed input.
    input: &'a [u8],
    /// The compressed output.
    output: &'a mut Vec<u8>,
    /// The number of bytes from the input that are encoded.
    cur: usize,
    /// The dictionary of previously encoded sequences.
    ///
    /// This is used to find duplicates in the stream so they are not written multiple times.
    ///
    /// Every four bytes are hashed, and in the resulting slot their position in the input buffer
    /// is placed. This way we can easily look up a candidate to back references.
    dict: [usize; DICTIONARY_SIZE],
}

impl<'a> Encoder<'a> {
    /// Go forward by some number of bytes.
    ///
    /// This will update the cursor and dictionary to reflect the now processed bytes.
    ///
    /// This returns `false` if all the input bytes are processed.
    fn go_forward(&mut self, steps: usize) -> bool {
        // Go over all the bytes we are skipping and update the cursor and dictionary.
        for _ in 0..steps {
            // Insert the cursor position into the dictionary.
            self.insert_cursor();
            // Increment the cursor.
            self.cur += 1;
        }

        // Return `true` if there's more to read.
        self.cur <= self.input.len()
    }

    /// Insert the batch under the cursor into the dictionary.
    fn insert_cursor(&mut self) {
        // Make sure that there is at least one batch remaining.
        if self.remaining_batch() {
            // Insert the cursor into the table.
            self.dict[self.get_cur_hash()] = self.cur;
        }
    }

    /// Check if there are any remaining batches.
    fn remaining_batch(&self) -> bool {
        self.cur + 4 < self.input.len()
    }

    /// Get the hash of the current four bytes below the cursor.
    ///
    /// This is guaranteed to be below `DICTIONARY_SIZE`.
    fn get_cur_hash(&self) -> usize {
        // Use PCG transform to generate a relatively good hash of the four bytes batch at the
        // cursor.
        let mut x = self.get_batch_at_cursor().wrapping_mul(0xa4d94a4f);
        let a = x >> 16;
        let b = x >> 30;
        x ^= a >> b;
        x = x.wrapping_mul(0xa4d94a4f);

        x as usize % DICTIONARY_SIZE
    }

    /// Read a 4-byte "batch" from some position.
    ///
    /// This will read a native-endian 4-byte integer from some position.
    fn get_batch(&self, n: usize) -> u32 {
        debug_assert!(self.remaining_batch(), "Reading a partial batch.");

        NativeEndian::read_u32(&self.input[n..])
    }

    /// Read the batch at the cursor.
    fn get_batch_at_cursor(&self) -> u32 {
        self.get_batch(self.cur)
    }

    /// Find a duplicate of the current batch.
    ///
    /// If any duplicate is found, a tuple `(position, size - 4)` is returned.
    fn find_duplicate(&self) -> Option<Duplicate> {
        // If there is no remaining batch, we return none.
        if !self.remaining_batch() {
            return None;
        }

        // Find a candidate in the dictionary by hashing the current four bytes.
        let candidate = self.dict[self.get_cur_hash()];

        // Three requirements to the candidate exists:
        // - The candidate is not the trap value (0xFFFFFFFF), which represents an empty bucket.
        // - We should not return a position which is merely a hash collision, so w that the
        //   candidate actually matches what we search for.
        // - We can address up to 16-bit offset, hence we are only able to address the candidate if
        //   its offset is less than or equals to 0xFFFF.
        if candidate != !0
           && self.get_batch(candidate) == self.get_batch_at_cursor()
           && self.cur - candidate <= 0xFFFF {
            // Calculate the "extension bytes", i.e. the duplicate bytes beyond the batch. These
            // are the number of prefix bytes shared between the match and needle.
            let ext = self.input[self.cur + 4..]
                .iter()
                .zip(&self.input[candidate + 4..])
                .take_while(|&(a, b)| a == b)
                .count();

            Some(Duplicate {
                offset: (self.cur - candidate) as u16,
                extra_bytes: ext,
            })
        } else { None }
    }

    /// Write an integer to the output in LSIC format.
    fn write_integer(&mut self, mut n: usize) {
        // Write the 0xFF bytes as long as the integer is higher than said value.
        while n >= 0xFF {
            n -= 0xFF;
            self.output.push(0xFF);
        }

        // Write the remaining byte.
        self.output.push(n as u8);
    }

    /// Read the block of the top of the stream.
    fn pop_block(&mut self) -> Block {
        // The length of the literals section.
        let mut lit = 0;

        loop {
            // Search for a duplicate.
            if let Some(dup) = self.find_duplicate() {
                // We found a duplicate, so the literals section is over...

                // Move forward. Note that `ext` is actually the steps minus 4, because of the
                // minimum matchlenght, so we need to add 4.
                self.go_forward(dup.extra_bytes + 4);

                return Block {
                    lit_len: lit,
                    dup: Some(dup),
                };
            }

            // Try to move forward.
            if !self.go_forward(1) {
                // We reached the end of the stream, and no duplicates section follows.
                return Block {
                    lit_len: lit,
                    dup: None,
                };
            }

            // No duplicates found yet, so extend the literals section.
            lit += 1;
        }
    }

    /// Complete the encoding into `self.output`.
    fn complete(&mut self) {
        // Construct one block at a time.
        loop {
            // The start of the literals section.
            let start = self.cur;

            // Read the next block into two sections, the literals and the duplicates.
            let block = self.pop_block();

            // Generate the higher half of the token.
            let mut token = if block.lit_len < 0xF {
                // Since we can fit the literals length into it, there is no need for saturation.
                (block.lit_len as u8) << 4
            } else {
                // We were unable to fit the literals into it, so we saturate to 0xF. We will later
                // write the extensional value through LSIC encoding.
                0xF0
            };

            // Generate the lower half of the token, the duplicates length.
            let dup_extra_len = block.dup.map_or(0, |x| x.extra_bytes);
            token |= if dup_extra_len < 0xF {
                // We could fit it in.
                dup_extra_len as u8
            } else {
                // We were unable to fit it in, so we default to 0xF, which will later be extended
                // by LSIC encoding.
                0xF
            };

            // Push the token to the output stream.
            self.output.push(token);

            // If we were unable to fit the literals length into the token, write the extensional
            // part through LSIC.
            if block.lit_len >= 0xF {
                self.write_integer(block.lit_len - 0xF);
            }

            // Now, write the actual literals.
            self.output.extend_from_slice(&self.input[start..start + block.lit_len]);

            if let Some(Duplicate { offset, .. }) = block.dup {
                // Wait! There's more. Now, we encode the duplicates section.

                // Push the offset in little endian.
                self.output.push(offset as u8);
                self.output.push((offset >> 8) as u8);

                // If we were unable to fit the duplicates length into the token, write the
                // extensional part through LSIC.
                if dup_extra_len >= 0xF {
                    self.write_integer(dup_extra_len - 0xF);
                }
            } else {
                break;
            }
        }
    }
}

/// Compress all bytes of `input` into `output`.
pub fn compress_into(input: &[u8], output: &mut Vec<u8>) {
    Encoder {
        input: input,
        output: output,
        cur: 0,
        dict: [!0; DICTIONARY_SIZE],
    }.complete();
}

/// Compress all bytes of `input`.
pub fn compress(input: &[u8]) -> Vec<u8> {
    // In most cases, the compression won't expand the size, so we set the input size as capacity.
    let mut vec = Vec::with_capacity(input.len());

    compress_into(input, &mut vec);

    vec
}
