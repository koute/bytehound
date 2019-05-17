use std::io;
use std::collections::HashMap;

use byteorder::{NativeEndian, WriteBytesExt};

use crate::data::{Allocation, Data, Operation};

#[derive(Default)]
struct Exporter {
    free_slots: Vec< usize >,
    slot_count: usize,
    slot_by_pointer: HashMap< u64, usize >,
    slot_by_index: Vec< usize >,
    operation_count: usize
}

impl Exporter {
    fn preprocess_alloc( &mut self, allocation: &Allocation ) {
        let slot = if self.free_slots.is_empty() {
            let slot = self.slot_count;
            self.slot_count += 1;
            slot
        } else {
            self.free_slots.pop().unwrap()
        };

        self.slot_by_pointer.insert( allocation.pointer, slot );
        self.slot_by_index.push( slot );
    }

    fn preprocess_dealloc( &mut self, allocation: &Allocation ) {
        let slot = self.slot_by_pointer.remove( &allocation.pointer ).unwrap();
        self.free_slots.push( slot );
        self.slot_by_index.push( slot );
    }

    fn preprocess_realloc( &mut self, new_allocation: &Allocation, old_allocation: &Allocation ) {
        let slot = self.slot_by_pointer.remove( &old_allocation.pointer ).unwrap();
        self.slot_by_pointer.insert( new_allocation.pointer, slot );
        self.slot_by_index.push( slot );
    }

    fn generate_alloc< T: io::Write >( mut output: T, slot: usize, allocation: &Allocation ) -> io::Result< () > {
        let timestamp = allocation.timestamp.as_usecs();
        output.write_u64::< NativeEndian >( 1 )?;
        output.write_u64::< NativeEndian >( slot as u64 )?;
        output.write_u64::< NativeEndian >( timestamp as u64 )?;
        output.write_u64::< NativeEndian >( allocation.size as u64 )?;
        Ok(())
    }

    fn generate_dealloc< T: io::Write >( mut output: T, slot: usize, allocation: &Allocation ) -> io::Result< () > {
        let timestamp = allocation.timestamp.as_usecs();
        output.write_u64::< NativeEndian >( 2 )?;
        output.write_u64::< NativeEndian >( slot as u64 )?;
        output.write_u64::< NativeEndian >( timestamp as u64 )?;
        output.write_u64::< NativeEndian >( 0 )?;
        Ok(())
    }

    fn generate_realloc< T: io::Write >( mut output: T, slot: usize, new_allocation: &Allocation ) -> io::Result< () > {
        let timestamp = new_allocation.timestamp.as_usecs();
        output.write_u64::< NativeEndian >( 3 )?;
        output.write_u64::< NativeEndian >( slot as u64 )?;
        output.write_u64::< NativeEndian >( timestamp as u64 )?;
        output.write_u64::< NativeEndian >( new_allocation.size as u64 )?;
        Ok(())
    }

    fn process< T: io::Write, F: Fn( &Allocation ) -> bool >( mut self, data: &Data, filter: F, mut output: T ) -> io::Result< () > {
        for operation in data.operations() {
            self.operation_count += 1;
            match operation {
                Operation::Allocation { allocation, .. } => {
                    if !filter( allocation ) {
                        continue;
                    }

                    self.preprocess_alloc( allocation );
                },
                Operation::Deallocation { allocation, .. } => {
                    if !filter( allocation ) {
                        continue;
                    }

                    self.preprocess_dealloc( allocation );
                },
                Operation::Reallocation { new_allocation, old_allocation, .. } => {
                    let is_new_ok = filter( new_allocation );
                    let is_old_ok = filter( old_allocation );

                    if is_new_ok && is_old_ok {
                        self.preprocess_realloc( new_allocation, old_allocation );
                    } else if is_new_ok {
                        self.preprocess_alloc( new_allocation );
                    } else if is_old_ok {
                        self.preprocess_dealloc( old_allocation );
                    }
                }
            }
        }

        output.write_u64::< NativeEndian >( self.slot_count as u64 )?;
        output.write_u64::< NativeEndian >( self.operation_count as u64 )?;

        for (operation, slot) in data.operations().zip( self.slot_by_index ) {
            match operation {
                Operation::Allocation { allocation, .. } => {
                    if !filter( allocation ) {
                        continue;
                    }

                    Self::generate_alloc( &mut output, slot, allocation )?;
                },
                Operation::Deallocation { allocation, .. } => {
                    if !filter( allocation ) {
                        continue;
                    }

                    Self::generate_dealloc( &mut output, slot, allocation )?;
                },
                Operation::Reallocation { new_allocation, old_allocation, .. } => {
                    let is_new_ok = filter( new_allocation );
                    let is_old_ok = filter( old_allocation );

                    if is_new_ok && is_old_ok {
                        Self::generate_realloc( &mut output, slot, new_allocation )?;
                    } else if is_new_ok {
                        Self::generate_alloc( &mut output, slot, new_allocation )?;
                    } else if is_old_ok {
                        Self::generate_dealloc( &mut output, slot, old_allocation )?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn export_as_replay< T: io::Write, F: Fn( &Allocation ) -> bool >( data: &Data, output: T, filter: F ) -> io::Result< () > {
    Exporter::default().process( data, filter, output )
}
