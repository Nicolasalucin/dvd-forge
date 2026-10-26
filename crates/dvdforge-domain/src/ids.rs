use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, specta::Type,
        )]
        #[serde(transparent)]
        #[specta(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_u128(value: u128) -> Self {
                Self(Uuid::from_u128(value))
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                // Tauri IPC can receive stale UI state containing empty IDs.
                // Never abort the whole command because of one missing UUID:
                // repair it with a fresh identifier instead.
                let value = Option::<String>::deserialize(deserializer)?;
                match value.as_deref().map(str::trim) {
                    None | Some("") => Ok(Self::new()),
                    Some(value) => Uuid::parse_str(value)
                        .map(Self)
                        .map_err(serde::de::Error::custom),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(value)?))
            }
        }
    };
}

id_type!(ProjectId);
id_type!(MenuId);
id_type!(EpisodeId);
id_type!(ButtonId);
id_type!(AssetId);
id_type!(ChapterId);
id_type!(JobId);
id_type!(PlanId);

/// Deserializes `Option<Id>` fields tolerantly: empty strings and `null`
/// become `None` instead of producing an invalid UUID error.
pub fn deserialize_optional_id<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr<Err = uuid::Error>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse::<T>()
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_strings() {
        let id = ButtonId::from_u128(42);
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"00000000-0000-0000-0000-00000000002a\"");
        let parsed: ButtonId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, id);
    }

    #[test]
    fn empty_required_ids_are_repaired() {
        let first: MenuId = serde_json::from_str("\"\"").expect("empty menu id");
        let second: MenuId = serde_json::from_str("\"  \"").expect("blank menu id");
        assert!(!first.as_uuid().is_nil());
        assert!(!second.as_uuid().is_nil());
        assert_ne!(first, second);
        let from_null: AssetId = serde_json::from_str("null").expect("null asset id");
        assert!(!from_null.as_uuid().is_nil());
    }

    #[test]
    fn empty_optional_ids_become_none() {
        #[derive(Debug, Deserialize)]
        struct OptionalId {
            #[serde(default, deserialize_with = "deserialize_optional_id")]
            value: Option<ButtonId>,
        }
        let parsed: OptionalId = serde_json::from_str(r#"{"value":""}"#).expect("empty optional");
        assert!(parsed.value.is_none());
        let parsed: OptionalId = serde_json::from_str(r#"{"value":null}"#).expect("null optional");
        assert!(parsed.value.is_none());
    }
}
