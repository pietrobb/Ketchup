//! Typed IDs. Every ID of the model is declared with [`typed_id!`](crate::typed_id), so all
//! of them serialize as a bare number and order the same way.

/// Declares a `pub struct $name(pub u64)` ID that serializes as its number.
#[macro_export]
macro_rules! typed_id {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

typed_id!(DocumentId);
typed_id!(DefinitionId);
typed_id!(FeatureId);
typed_id!(NodeId);
