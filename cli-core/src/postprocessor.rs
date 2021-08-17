use std::io::{self, Read, Write};
use std::ffi::OsStr;
use std::u64;
use std::borrow::Cow;

use ahash::AHashSet as HashSet;
use string_interner::Symbol;

use nwind::{
    DebugInfoIndex
};
use common::speedy::{
    Writable,
};

use common::event::{
    Event,
    AllocBody,
    HeaderBody,
};

use common::lz4_stream::{
    Lz4Writer
};

use crate::loader::Loader;
use crate::reader::parse_events;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Anonymize {
    None,
    Partial,
    Full
}

fn anonymize_header( anonymize: Anonymize, header: &mut HeaderBody ) {
    match anonymize {
        Anonymize::None => {},
        Anonymize::Partial => {
            if let Some( index ) = header.executable.iter().rev().position( |&byte| byte == b'/' ) {
                header.executable = header.executable[ header.executable.len() - index.. ].into();
            }

            header.cmdline = header.executable.clone();
        },
        Anonymize::Full => {
            header.executable = b"program"[..].to_owned();
            header.cmdline = header.executable.clone();
        }
    }
}

struct PathAnonymizer {
    prefix: &'static str,
    counter: usize
}

impl PathAnonymizer {
    fn new( prefix: &'static str ) -> Self {
        Self {
            prefix,
            counter: 0
        }
    }

    fn anonymize< 'a >( &mut self, anonymize: Anonymize, string: &'a str ) -> Cow< 'a, str > {
        match anonymize {
            Anonymize::None => string.into(),
            Anonymize::Partial => {
                if let Some( index ) = string.as_bytes().iter().rev().position( |&byte| byte == b'/' ) {
                    string[ string.len() - index.. ].into()
                } else {
                    string.into()
                }
            },
            Anonymize::Full => {
                let counter = self.counter;
                self.counter += 1;

                format!( "{}{}", self.prefix, counter ).into()
            }
        }
    }
}

#[derive(Default)]
struct FunctionAnonymizer {
    counter: usize
}

impl FunctionAnonymizer {
    fn anonymize< 'a >( &mut self, anonymize: Anonymize, string: &'a str ) -> Cow< 'a, str > {
        if anonymize != Anonymize::Full {
            string.into()
        } else {
            let counter = self.counter;
            self.counter += 1;

            format!( "fn_{}", counter ).into()
        }
    }
}

pub fn postprocess< F, G, D, I  >( ifp: F, ofp: G, debug_symbols: I, anonymize: Anonymize ) -> Result< (), io::Error >
    where F: Read + Send + 'static,
          G: Write,
          D: AsRef< OsStr >,
          I: IntoIterator< Item = D >
{
    let mut ofp = Lz4Writer::new( ofp );
    let (mut header, event_stream) = parse_events( ifp )?;

    let mut debug_info_index = DebugInfoIndex::new();
    for path in debug_symbols {
        debug_info_index.add( path.as_ref() );
    }

    let mut loader = Loader::new( header.clone(), debug_info_index );
    anonymize_header( anonymize, &mut header );
    Event::Header( header ).write_to_stream( &mut ofp )?;

    let mut anonymizer_library = PathAnonymizer::new( "lib_" );
    let mut anonymizer_source = PathAnonymizer::new( "src_" );
    let mut anonymizer_function = FunctionAnonymizer::default();

    let mut frames = Vec::new();
    let mut frames_to_write = Vec::new();
    let mut emitted_strings = HashSet::new();
    let mut expected_backtrace_id = 0;
    let mut expected_frame_id = 0;
    for event in event_stream {
        let mut event = event?;
        let mut process = false;
        let mut is_backtrace = false;
        let mut write = true;
        match event {
            Event::Backtrace { .. } |
            Event::Backtrace32 { .. } => {
                is_backtrace = true;
                write = false;
            },
            Event::PartialBacktrace { .. } |
            Event::PartialBacktrace32 { .. } => {
                is_backtrace = true;
                write = false;
            },
            Event::Alloc { allocation: AllocBody { ref mut backtrace, .. }, .. } |
            Event::AllocEx { allocation: AllocBody { ref mut backtrace, .. }, .. } |
            Event::Realloc { allocation: AllocBody { ref mut backtrace, .. }, .. } |
            Event::ReallocEx { allocation: AllocBody { ref mut backtrace, .. }, .. } |
            Event::Free { ref mut backtrace, .. } |
            Event::FreeEx { ref mut backtrace, .. } |
            Event::MemoryMap { ref mut backtrace, .. } |
            Event::MemoryUnmap { ref mut backtrace, .. } |
            Event::Mallopt { ref mut backtrace, .. } |
            Event::GroupStatistics { ref mut backtrace, .. } => {
                if let Some( target_backtrace ) = loader.lookup_backtrace( *backtrace ) {
                    *backtrace = target_backtrace.raw() as _;
                } else {
                    *backtrace = u64::MAX;
                }
            },

            Event::File { ref mut contents, .. } if contents.starts_with( b"\x7FELF" ) => {
                process = true;
                write = false;
            },

            Event::File { .. } => {
                process = true;
                if anonymize != Anonymize::None {
                    write = false;
                }
            },
            Event::Header( ref mut body ) => {
                anonymize_header( anonymize, body );
            },
            Event::MemoryDump { .. } => {
                if anonymize != Anonymize::None {
                    write = false;
                }
            },
            Event::Marker { .. } => {},
            Event::Environ { .. } => {
                if anonymize != Anonymize::None {
                    write = false;
                }
            },
            Event::WallClock { .. } => {},
            Event::String { .. } => {},
            Event::DecodedFrame { .. } => {},
            Event::DecodedBacktrace { .. } => {}
        }

        if write {
            event.write_to_stream( &mut ofp )?;
        }

        if is_backtrace {
            frames.clear();
            frames_to_write.clear();

            let backtrace_id = loader.process_backtrace_event( event, |frame_id, is_new| {
                frames.push( frame_id as u32 );
                if is_new {
                    frames_to_write.push( frame_id );
                }
            });

            if backtrace_id.is_none() {
                assert!( frames.is_empty() );
                assert!( frames_to_write.is_empty() );
            }

            for frame_id in frames_to_write.drain( .. ) {
                let frame = loader.get_frame( frame_id ).clone();
                macro_rules! intern {
                    ($value:expr, $anonymizer:ident) => {
                        if let Some( id ) = $value {
                            let raw_id = id.to_usize() as u32;
                            if !emitted_strings.contains( &id ) {
                                emitted_strings.insert( id );
                                let string = loader.interner().resolve( id ).unwrap();
                                Event::String {
                                    id: raw_id,
                                    string: $anonymizer.anonymize( anonymize, string )
                                }.write_to_stream( &mut ofp )?;
                            }

                            raw_id
                        } else {
                            0xFFFFFFFF
                        }
                    }
                }

                let library = intern!( frame.library(), anonymizer_library );
                let source = intern!( frame.source(), anonymizer_source );

                let raw_function;
                let function;
                if anonymize == Anonymize::Full {
                    raw_function = intern!( frame.raw_function(), anonymizer_function );
                    function = 0xFFFFFFFF;
                } else {
                    raw_function = intern!( frame.raw_function(), anonymizer_function );
                    function = intern!( frame.function(), anonymizer_function );
                }

                assert_eq!( frame_id, expected_frame_id );
                expected_frame_id += 1;

                Event::DecodedFrame {
                    address: frame.address().raw(),
                    library,
                    raw_function,
                    function,
                    source,
                    line: frame.line().unwrap_or( 0xFFFFFFFF ),
                    column: frame.column().unwrap_or( 0xFFFFFFFF ),
                    is_inline: frame.is_inline()
                }.write_to_stream( &mut ofp )?;
            }

            if let Some( backtrace_id ) = backtrace_id {
                assert_eq!( backtrace_id.raw(), expected_backtrace_id );
                expected_backtrace_id += 1;

                Event::DecodedBacktrace {
                    frames: (&frames).into()
                }.write_to_stream( &mut ofp )?;
            }
        } else if process {
            loader.process( event );
        }
    }

    Ok(())
}
