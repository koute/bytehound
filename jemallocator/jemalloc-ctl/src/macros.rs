//! Utility macros

macro_rules! types {
    ($id:ident[ str: $byte_string:expr, $mib:ty, $name_to_mib:ident ]  |
     docs: $(#[$doc:meta])*
     mib_docs: $(#[$doc_mib:meta])*
    ) => {
        paste::paste! {
            $(#[$doc])*
            #[allow(non_camel_case_types)]
            pub struct $id;

            impl $id {
                const NAME: &'static crate::keys::Name = {
                    union U<'a> {
                        bytes: &'a [u8],
                        name: &'a crate::keys::Name
                    }

                    unsafe { U { bytes: $byte_string }.name }
                };
                /// Returns Management Information Base (MIB)
                ///
                /// This value can be used to access the key without doing string lookup.
                pub fn mib() -> crate::error::Result<[<$id _mib>]> {
                    Ok([<$id _mib>](Self::NAME.$name_to_mib()?))
                }

                /// Key [`::keys::Name`].
                pub fn name() -> &'static crate::keys::Name {
                    Self::NAME
                }
            }

            $(#[$doc_mib])*
            #[repr(transparent)]
            #[derive(Copy, Clone)]
            #[allow(non_camel_case_types)]
            pub struct [<$id _mib>](pub crate::keys::$mib);
        }
    };
}

/// Read
macro_rules! r {
    ($id:ident => $ret_ty:ty) => {
        paste::paste! {
            impl $id {
                /// Reads value using string API.
                pub fn read() -> crate::error::Result<$ret_ty> {
                    use crate::keys::Access;
                    Self::NAME.read()
                }
            }

            impl [<$id _mib>] {
                /// Reads value using MIB API.
                pub fn read(self) -> crate::error::Result<$ret_ty> {
                    use crate::keys::Access;
                    self.0.read()
                }
            }

            #[cfg(test)]
            #[test]
            #[cfg(not(target_arch = "mips64el"))]
            #[allow(unused)]
            fn [<$id _read_test>]() {
                match stringify!($id) {
                    "background_thread" |
                    "max_background_threads"
                    if cfg!(target_os = "macos") => return,
                    _ => (),
                }

                let a = $id::read().unwrap();

                let mib = $id::mib().unwrap();
                let b = mib.read().unwrap();

                #[cfg(feature = "use_std")]
                println!(
                    concat!(
                        stringify!($id),
                        " (read): \"{}\" - \"{}\""),
                    a, b
                );
            }
        }
    };
}

/// Write
macro_rules! w {
    ($id:ident => $ret_ty:ty) => {
        paste::paste! {
            impl $id {
                /// Writes `value` using string API.
                pub fn write(value: $ret_ty) -> crate::error::Result<()> {
                    use crate::keys::Access;
                    Self::NAME.write(value)
                }
            }

            impl [<$id _mib>] {
                /// Writes `value` using MIB API.
                pub fn write(self, value: $ret_ty) -> crate::error::Result<()> {
                    use crate::keys::Access;
                    self.0.write(value)
                }
            }

            #[cfg(test)]
            #[test]
            #[cfg(not(target_arch = "mips64el"))]
            fn [<$id _write_test>]() {
                match stringify!($id) {
                    "background_thread" |
                    "max_background_threads"
                        if cfg!(target_os = "macos") => return,
                    _ => (),
                }

                let _ = $id::write($ret_ty::default()).unwrap();

                let mib = $id::mib().unwrap();
                let _ = mib.write($ret_ty::default()).unwrap();

                #[cfg(feature = "use_std")]
                println!(
                    concat!(
                        stringify!($id),
                        " (write): \"{}\""),
                    $ret_ty::default()
                );

            }
        }
    };
}

/// Update
macro_rules! u {
    ($id:ident  => $ret_ty:ty) => {
        paste::paste! {
            impl $id {
                /// Updates key to `value` returning its old value using string API.
                pub fn update(value: $ret_ty) -> crate::error::Result<$ret_ty> {
                    use crate::keys::Access;
                    Self::NAME.update(value)
                }
            }

            impl [<$id _mib>] {
                /// Updates key to `value` returning its old value using MIB API.
                pub fn update(self, value: $ret_ty) -> crate::error::Result<$ret_ty> {
                    use crate::keys::Access;
                    self.0.update(value)
                }
            }

            #[cfg(test)]
            #[test]
            #[cfg(not(target_arch = "mips64el"))]
            #[allow(unused)]
            fn [<$id _update_test>]() {
                match stringify!($id) {
                    "background_thread" |
                    "max_background_threads"
                        if cfg!(target_os = "macos") => return,
                    _ => (),
                }

                let a = $id::update($ret_ty::default()).unwrap();

                let mib = $id::mib().unwrap();
                let b = mib.update($ret_ty::default()).unwrap();

                #[cfg(feature = "use_std")]
                println!(
                    concat!(
                        stringify!($id),
                        " (update): (\"{}\", \"{}\") - \"{}\""),
                    a, b, $ret_ty::default()
                );
            }
        }
    };
}

/// Creates a new option
macro_rules! option {
    ($id:ident[ str: $byte_string:expr, $mib:ty, $name_to_mib:ident ] => $ret_ty:ty |
     ops: $($ops:ident),* |
     docs:
     $(#[$doc:meta])*
     mib_docs:
     $(#[$doc_mib:meta])*
    ) => {
        types! {
            $id[ str: $byte_string, $mib, $name_to_mib ] |
            docs: $(#[$doc])*
            mib_docs: $(#[$doc_mib])*
        }
        $(
            $ops!($id => $ret_ty);
        )*
    };
    // Non-string option:
    ($id:ident[ str: $byte_string:expr, non_str: $mib_len:expr ] => $ret_ty:ty |
     ops: $($ops:ident),* |
     docs:
     $(#[$doc:meta])*
     mib_docs:
     $(#[$doc_mib:meta])*
    ) => {
        option! {
            $id[ str: $byte_string, Mib<[usize; $mib_len]>, mib ] => $ret_ty |
            ops: $($ops),* |
            docs: $(#[$doc])*
            mib_docs: $(#[$doc_mib])*
        }
    };
    // String option:
    ($id:ident[ str: $byte_string:expr, str: $mib_len:expr ] => $ret_ty:ty |
     ops: $($ops:ident),* |
     docs:
     $(#[$doc:meta])*
     mib_docs:
     $(#[$doc_mib:meta])*
    ) => {
        option! {
            $id[ str: $byte_string, MibStr<[usize; $mib_len]>, mib_str ] => $ret_ty |
            ops: $($ops),* |
            docs: $(#[$doc])*
            mib_docs: $(#[$doc_mib])*
        }
    };
}
