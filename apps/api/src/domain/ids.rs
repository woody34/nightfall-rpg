//! Declarative macro for UUID-backed identifier newtypes (docs/engineering/rust-guidelines.md
//! section 2). One definition, so every id behaves identically.

/// Declares `pub struct Name(Uuid)`: private field, `new` (UUID v7, time-ordered so it indexes
/// well in Postgres), `from_uuid`, `as_uuid`, `Default`, `Display`, `FromStr`, `From`
/// conversions to and from `Uuid`, and a transparent serde representation (a bare UUID string).
///
/// `SeaORM` conversions are deliberately not emitted here: the domain layer stays free of
/// persistence crates. See `uuid_id_sea_orm!` in `infrastructure::postgres`.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(::uuid::Uuid);

        impl $name {
            /// Generates a fresh time-ordered id.
            #[must_use]
            pub fn new() -> Self {
                Self(::uuid::Uuid::now_v7())
            }

            /// Wraps an existing UUID (from storage, a verified token or the wire).
            #[must_use]
            pub const fn from_uuid(id: ::uuid::Uuid) -> Self {
                Self(id)
            }

            /// The underlying UUID.
            #[must_use]
            pub const fn as_uuid(&self) -> ::uuid::Uuid {
                self.0
            }
        }

        impl ::std::default::Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::uuid::Error;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                ::uuid::Uuid::parse_str(s).map(Self)
            }
        }

        impl ::std::convert::From<::uuid::Uuid> for $name {
            fn from(id: ::uuid::Uuid) -> Self {
                Self(id)
            }
        }

        impl ::std::convert::From<$name> for ::uuid::Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

pub(crate) use uuid_id;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use uuid::Uuid;

    uuid_id!(
        /// Test-only id.
        ProbeId
    );

    #[test]
    fn new_is_time_ordered_and_unique() {
        let (a, b) = (ProbeId::new(), ProbeId::default());
        assert_ne!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 7);
        assert!(a <= b);
    }

    #[test]
    fn round_trips_through_uuid_display_and_from_str() {
        let uuid = Uuid::from_u128(0x0190_a7e2_6f4c_7c3b_9f1a_3f4a_5b6c_7d8e);
        let id = ProbeId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
        assert_eq!(ProbeId::from(uuid), id);
        assert_eq!(Uuid::from(id), uuid);
        assert_eq!(id.to_string(), uuid.to_string());
        assert_eq!(id.to_string().parse::<ProbeId>().unwrap(), id);
        assert!("not-a-uuid".parse::<ProbeId>().is_err());
        assert!("".parse::<ProbeId>().is_err());
    }

    #[test]
    fn serde_is_a_bare_uuid_string() {
        let id = ProbeId::from_uuid(Uuid::nil());
        let json = serde_json::to_value(id).unwrap();
        assert_eq!(json, serde_json::json!("00000000-0000-0000-0000-000000000000"));
        assert_eq!(serde_json::from_value::<ProbeId>(json).unwrap(), id);
    }

    #[test]
    fn orders_like_the_underlying_uuid() {
        let (lo, hi) =
            (ProbeId::from_uuid(Uuid::from_u128(1)), ProbeId::from_uuid(Uuid::from_u128(2)));
        assert!(lo < hi);
        assert_eq!(lo.max(hi), hi);
    }
}
