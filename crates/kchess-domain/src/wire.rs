//! Declaring a string-literal union once: the enum, its wire values (serde), every value in order
//! and the value a wire string names. The contracts declare theirs with the same macros.

/// A string-literal union: an enum serialised as the given strings, with `ALL`, `as_str` and
/// `from_key`.
#[macro_export]
macro_rules! union {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$variant_meta:meta])* $variant:ident = $wire:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(
            ::serde::Serialize, ::serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq,
            PartialOrd, Ord, Hash,
        )]
        pub enum $name {
            $($(#[$variant_meta])* #[serde(rename = $wire)] $variant,)+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &[$name] = &[$($name::$variant),+];

            /// Its wire value.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $wire,)+
                }
            }

            /// The value a wire string names; `None` for any other string.
            pub fn from_key(key: &str) -> Option<$name> {
                match key {
                    $($wire => Some($name::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

/// A string-literal union with the array of its wire values under `$konst`.
#[macro_export]
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident, $konst:ident {
        $($(#[$variant_meta:meta])* $variant:ident = $wire:literal),+ $(,)?
    }) => {
        $crate::union!($(#[$meta])* $name { $($(#[$variant_meta])* $variant = $wire),+ });
        /// The wire values of the union, in declaration order.
        pub const $konst: &[&str] = &[$($wire),+];
    };
}
