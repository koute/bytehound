// This file is based on elfhacks by Pyry Haulos.

/* elfhacks.h -- Various ELF run-time hacks
  version 0.4.1, March 9th, 2008

  Copyright (C) 2007-2008 Pyry Haulos

  This software is provided 'as-is', without any express or implied
  warranty.  In no event will the authors be held liable for any damages
  arising from the use of this software.

  Permission is granted to anyone to use this software for any purpose,
  including commercial applications, and to alter it and redistribute it
  freely, subject to the following restrictions:

  1. The origin of this software must not be misrepresented; you must not
     claim that you wrote the original software. If you use this software
     in a product, an acknowledgment in the product documentation would be
     appreciated but is not required.
  2. Altered source versions must be plainly marked as such, and must not be
     misrepresented as being the original software.
  3. This notice may not be removed or altered from any source distribution.

  Pyry Haulos <pyry.haulos@gmail.com>
*/

use libc::{c_char, c_void, size_t};
use std::ops::ControlFlow;
use std::ffi::CStr;

const STT_GNU_IFUNC: u8 = 10;

#[cfg(target_pointer_width = "32")]
type ProgramHeader = libc::Elf32_Phdr;

#[cfg(target_pointer_width = "64")]
type ProgramHeader = libc::Elf64_Phdr;

#[cfg(target_pointer_width = "32")]
type Symbol = libc::Elf32_Sym;

#[cfg(target_pointer_width = "64")]
type Symbol = libc::Elf64_Sym;

#[repr(C)]
pub struct Dynamic {
    tag: usize,
    value: usize,
}

pub struct ObjectInfo< 'a > {
    pub address: usize,
    pub name: &'a [u8],
    program_headers: &'a [ProgramHeader]
}

impl< 'a > ObjectInfo< 'a > {
    pub fn each< F, R >( mut callback: F ) -> Option< R > where F: FnMut( ObjectInfo ) -> ControlFlow< R, () > {
        Self::each_impl( &mut callback )
    }

    pub fn name_contains( &self, substr: impl AsRef< [u8] > ) -> bool {
        let substr = substr.as_ref();
        self.name.windows( substr.len() ).any( |window| window == substr )
    }

    fn each_impl< R >( callback: &mut dyn FnMut( ObjectInfo ) -> ControlFlow< R, () > ) -> Option< R > {
        struct Arg< 'a, R > {
            callback: &'a mut dyn FnMut( ObjectInfo ) -> ControlFlow< R, () >,
            result: Option< R >
        }

        let mut arg = Arg {
            callback,
            result: None
        };

        extern "C" fn callback_static< R >( info: *mut libc::dl_phdr_info, _size: size_t, arg: *mut c_void ) -> i32 {
            let info = unsafe { &*info };
            let arg = unsafe { &mut *(arg as *mut Arg< R >) };
            let info = ObjectInfo {
                address: info.dlpi_addr as usize,
                name:
                    if info.dlpi_name.is_null() {
                        &[]
                    } else {
                        unsafe {
                            CStr::from_ptr( info.dlpi_name ).to_bytes()
                        }
                    },
                program_headers:
                    if info.dlpi_phnum == 0 {
                        &[]
                    } else {
                        unsafe {
                            std::slice::from_raw_parts( info.dlpi_phdr, info.dlpi_phnum as usize )
                        }
                    }
            };

            let result = (arg.callback)( info );
            match result {
                ControlFlow::Break( result ) => {
                    arg.result = Some( result );
                    1
                },
                ControlFlow::Continue(()) => {
                    0
                }
            }
        }

        unsafe {
            libc::dl_iterate_phdr( Some( callback_static::< R > ), (&mut arg) as *mut Arg< R > as *mut c_void );
        }

        arg.result
    }

    pub fn dlsym( &self, name: impl AsRef< [u8] > ) -> Option< *mut c_void > {
        self.dlsym_impl( name.as_ref() )
    }

    fn dlsym_impl( &self, name: &[u8] ) -> Option< *mut c_void > {
        let mut dt_hash = None;
        let mut dt_strtab = None;
        let mut dt_symtab = None;
        let mut dt_gnu_hash = None;
        for dynamic in self.dynamic() {
            match dynamic.tag {
                4 if self.check_address( dynamic.value ) => dt_hash = Some( dynamic.value as *const u32 ),
                5 if self.check_address( dynamic.value ) => dt_strtab = Some( dynamic.value as *const u8 ),
                6 if self.check_address( dynamic.value ) => dt_symtab = Some( dynamic.value as *const Symbol ),
                0x6fff_fef5 if self.check_address( dynamic.value ) => dt_gnu_hash = Some( dynamic.value as *const u32 ),
                _ => {}
            }
        }

        if let (Some( dt_strtab ), Some( dt_symtab )) = (dt_strtab, dt_symtab) {
            let result_gnu_hash = dt_gnu_hash.and_then( |dt_gnu_hash| unsafe {
                self.dlsym_gnu_hash( dt_strtab, dt_symtab, dt_gnu_hash, name )
            });

            if result_gnu_hash.is_some() && !cfg!( test ) {
                return result_gnu_hash;
            }

            let result_elf_hash = dt_hash.and_then( |dt_hash| unsafe {
                self.dlsym_elf_hash( dt_strtab, dt_symtab, dt_hash, name )
            });

            if cfg!( test ) {
                assert_eq!( result_gnu_hash, result_elf_hash );
            }

            return result_elf_hash;
        }

        None
    }

    unsafe fn dlsym_gnu_hash( &self, dt_strtab: *const u8, dt_symtab: *const Symbol, dt_gnu_hash: *const u32, name: &[u8] ) -> Option< *mut c_void > {
        fn calculate_hash( name: &[u8] ) -> u32 {
            let mut hash: u32 = 5381;
            for &byte in name {
                hash = ( hash << 5 ).wrapping_add( hash ).wrapping_add( byte as u32 )
            }

            hash
        }

        if *dt_gnu_hash == 0 {
            return None;
        }

        let nbuckets: u32 = *dt_gnu_hash;
        let symbias: u32 = *dt_gnu_hash.add( 1 );
        let bitmask_nwords: u32 = *dt_gnu_hash.add( 2 );
        let bitmask_idxbits: u32 = bitmask_nwords - 1;
        let shift: u32 = *dt_gnu_hash.add( 3 );
        let bitmask: *const usize = dt_gnu_hash.add( 4 ).cast();
        let buckets: *const u32 = dt_gnu_hash.add( 4 + (std::mem::size_of::< usize >() / 4) * bitmask_nwords as usize );
        let chain_zero: *const u32 = buckets.wrapping_add( (nbuckets as usize).wrapping_sub( symbias as usize ) );

        let hash: u32 = calculate_hash( name );

        let bitmask_word: usize = *bitmask.add((hash as usize / (std::mem::size_of::< usize >() * 8)) & (bitmask_idxbits as usize));
        let mask = (std::mem::size_of::< usize >() as u32) * 8 - 1;
        let hashbit1: u32 = hash & mask;
        let hashbit2: u32 = (hash >> shift) & mask;

        if ((bitmask_word >> hashbit1) & (bitmask_word >> hashbit2) & 1) == 0 {
            return None;
        }

        let bucket = *buckets.add( (hash % nbuckets) as usize );
        if bucket == 0 {
            return None;
        }

        let mut hasharr: *const u32 = chain_zero.add( bucket as usize );
        loop {
            if ((*hasharr ^ hash) >> 1) == 0 {
                let symtab_offset = ((hasharr as usize) - (chain_zero as usize)) / std::mem::size_of::< u32 >();
                if let Some( pointer ) = self.resolve_symbol( dt_strtab, dt_symtab, symtab_offset, name ) {
                    return Some( pointer );
                }
            }

            if (*hasharr & 1) != 0 {
                break;
            }

            hasharr = hasharr.add( 1 );
        }

        None
    }

    unsafe fn dlsym_elf_hash( &self, dt_strtab: *const u8, dt_symtab: *const Symbol, dt_hash: *const u32, name: &[u8] ) -> Option< *mut c_void > {
        fn calculate_hash( name: &[u8] ) -> usize {
            let mut hash: usize = 0;
            for &byte in name {
                hash = (hash << 4).wrapping_add( byte as usize );
                let tmp = hash & 0xf0000000;
                if tmp != 0 {
                    hash = hash ^ (tmp >> 24);
                    hash = hash ^ tmp;
                }
            }
            hash
        }

        if *dt_hash == 0 {
            return None;
        }


        let hash: usize = calculate_hash( name );
        let bucket_idx = *dt_hash.add( 2 + (hash % (*dt_hash as usize)) ) as usize;
        let chain: *const u32 = dt_hash.add( 2 + (*dt_hash as usize) + bucket_idx );

        std::iter::once( bucket_idx as usize )
            .chain( (0..).map( |index| (*chain.add( index )) as usize ).take_while( |&offset| offset != 0 ) )
            .flat_map( |offset| self.resolve_symbol( dt_strtab, dt_symtab, offset, name ) )
            .next()
    }

    unsafe fn resolve_symbol( &self, dt_strtab: *const u8, dt_symtab: *const Symbol, symtab_offset: usize, expected_name: &[u8] ) -> Option< *mut c_void > {
        let sym = &*dt_symtab.add( symtab_offset );
        if sym.st_name == 0 {
            return None;
        }

        let symbol_name = CStr::from_ptr( dt_strtab.add( sym.st_name as usize ) as *const c_char ).to_bytes();
        if symbol_name != expected_name {
            return None;
        }

        let mut address = (self.address + sym.st_value as usize) as *mut c_void;
        if sym.st_info & 0b1111 == STT_GNU_IFUNC {
            let resolver: extern "C" fn() -> *mut c_void = std::mem::transmute( address );
            address = resolver();
        }

        Some( address )
    }

    fn check_address( &self, address: usize ) -> bool {
        let address = address as u64;
        self.program_headers.iter().any( |header| {
            header.p_type == libc::PT_LOAD &&
            (address < (header.p_memsz as u64) + (header.p_vaddr as u64) + (self.address as u64)) &&
            (address >= (header.p_vaddr as u64) + (self.address as u64))
        })
    }

    fn dynamic( &self ) -> impl Iterator< Item = &Dynamic > {
        let mut dynamic = self.program_headers.iter()
            .find( |header| header.p_type == libc::PT_DYNAMIC )
            .map( |header| (header.p_vaddr as usize + self.address) as *const Dynamic )
            .unwrap_or( std::ptr::null() );

        std::iter::from_fn( move ||
            unsafe {
                if dynamic.is_null() || (*dynamic).tag == 0 {
                    return None;
                }

                let out = dynamic;
                dynamic = dynamic.add( 1 );
                Some( &*out )
            }
        )
    }
}

#[test]
fn test_dlsym() {
    let result = ObjectInfo::each( |info| {
        if info.name_contains( "libc.so" ) {
            assert!( info.dlsym( "gettimeofday" ).is_some() );
            assert!( info.dlsym( "malloc" ).is_some() );
            assert!( info.dlsym( "sleep" ).is_some() );

            unsafe {
                let strlen: extern "C" fn( *const u8 ) -> usize = std::mem::transmute( info.dlsym( "strlen" ).unwrap() );
                assert_eq!( strlen( b"foobar\0".as_ptr() ), 6 );

                let isalnum: extern "C" fn( u32 ) -> u32 = std::mem::transmute( info.dlsym( "isalnum" ).unwrap() );
                for byte in 0..=255 {
                    assert_eq!( isalnum( byte as u32 ), libc::isalnum( byte as _ ) as _ );
                }
            }

            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    });

    assert!( result.is_some() );
}
