use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

pub fn parse(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UnambiguousValue>(bytes).map(|value| value.0)
}

struct UnambiguousValue(Value);

impl<'de> Deserialize<'de> for UnambiguousValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UnambiguousValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object fields")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Bool(value)))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Number(value.into())))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Number(value.into())))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| UnambiguousValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::String(value.to_owned())))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::String(value)))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Null))
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UnambiguousValue>()? {
                    values.push(value.0);
                }
                Ok(UnambiguousValue(Value::Array(values)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate JSON field: {key}")));
                    }
                    let value = object.next_value::<UnambiguousValue>()?;
                    values.insert(key, value.0);
                }
                Ok(UnambiguousValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn conflicting_fields_are_rejected_at_every_depth() {
        for bytes in [
            br#"{"id":1,"id":2}"#.as_slice(),
            br#"{"params":{"overwrite":false,"overwrite":true}}"#,
            br#"{"params":{"discard_unsaved":false,"discard_unsaved":true}}"#,
            br#"{"program":{"operations":[{"operation":"transform","operation":"delete"}]}}"#,
            br#"{"params":{"overwrite":false,"over\u0077rite":true}}"#,
        ] {
            assert!(
                parse(bytes)
                    .unwrap_err()
                    .to_string()
                    .contains("duplicate JSON field")
            );
        }
    }

    #[test]
    fn unique_fields_and_numeric_types_match_standard_json() {
        let bytes = br#"{"null":null,"bool":true,"string":"x","integer":18446744073709551615,"negative":-2,"number":1.25,"array":[{"id":1},{"id":2}]}"#;
        assert_eq!(
            parse(bytes).unwrap(),
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()
        );
    }

    #[test]
    fn malformed_nonfinite_and_trailing_json_are_rejected() {
        for bytes in [b"NaN".as_slice(), b"1e10000", b"{} {}", b"{", b"[Infinity]"] {
            assert!(parse(bytes).is_err());
        }
    }
}
