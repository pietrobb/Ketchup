//! Generate the entire wire schema from the authoritative serde CAD types.
//! No second hand-maintained list of CAD variants is allowed here. Unsupported
//! source syntax/serde attributes fail the build rather than publish a subset.
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, env, fs, path::PathBuf};
use syn::{Fields, GenericArgument, Item, PathArguments, Type};

#[derive(Default)]
struct Serde {
    tag: Option<String>,
    snake: bool,
    default: bool,
    deny_unknown: bool,
}

fn attributes(attrs: &[syn::Attribute]) -> Serde {
    let mut result = Serde::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tag") {
                result.tag = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("rename_all") {
                assert_eq!(meta.value()?.parse::<syn::LitStr>()?.value(), "snake_case");
                result.snake = true;
            } else if meta.path.is_ident("deny_unknown_fields") {
                result.deny_unknown = true;
            } else if meta.path.is_ident("default") {
                result.default = true;
            } else if meta.path.is_ident("skip_serializing_if") {
                assert_eq!(
                    meta.value()?.parse::<syn::LitStr>()?.value(),
                    "Option::is_none"
                );
            } else {
                panic!("unsupported CAD schema serde attribute");
            }
            Ok(())
        })
        .expect("parse CAD serde attributes");
    }
    result
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

struct Generator {
    items: BTreeMap<String, Item>,
    definitions: Map<String, Value>,
}
impl Generator {
    fn ty(&mut self, ty: &Type) -> Value {
        match ty {
            Type::Array(array) => {
                let syn::Expr::Lit(length) = &array.len else {
                    panic!("nonliteral array bound")
                };
                let syn::Lit::Int(length) = &length.lit else {
                    panic!("noninteger array bound")
                };
                let n: usize = length.base10_parse().unwrap();
                json!({"type":"array", "items":self.ty(&array.elem), "minItems":n,"maxItems":n})
            }
            Type::Path(path) => {
                assert!(path.qself.is_none());
                let segment = path.path.segments.last().unwrap();
                let name = segment.ident.to_string();
                match name.as_str() {
                    "String" => json!({"type":"string"}),
                    "bool" => json!({"type":"boolean"}),
                    "f64" => json!({"type":"number"}),
                    "u64" => json!({"type":"integer","minimum":0,"maximum":u64::MAX}),
                    "u32" => json!({"type":"integer","minimum":0,"maximum":u32::MAX}),
                    "u8" => json!({"type":"integer","minimum":0,"maximum":u8::MAX}),
                    "Vec" | "Option" => {
                        let PathArguments::AngleBracketed(args) = &segment.arguments else {
                            panic!("missing generic")
                        };
                        assert_eq!(args.args.len(), 1);
                        let GenericArgument::Type(inner) = &args.args[0] else {
                            panic!("unsupported generic")
                        };
                        let inner = self.ty(inner);
                        if name == "Vec" {
                            json!({"type":"array","items":inner})
                        } else {
                            json!({"anyOf":[inner,{"type":"null"}]})
                        }
                    }
                    _ => {
                        self.named(&name);
                        json!({"$ref":format!("#/$defs/{name}")})
                    }
                }
            }
            _ => panic!("unsupported CAD schema field type"),
        }
    }

    fn fields(&mut self, fields: &Fields, tag: Option<(&str, &str)>) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        if let Some((tag, value)) = tag {
            properties.insert(tag.to_owned(), json!({"const":value}));
            required.push(tag.to_owned());
        }
        assert!(
            !matches!(fields, Fields::Unnamed(_)),
            "tuple CAD records unsupported"
        );
        for field in fields {
            let name = field.ident.as_ref().unwrap().to_string();
            let attrs = attributes(&field.attrs);
            let optional = matches!(&field.ty, Type::Path(p) if p.path.segments.last().unwrap().ident == "Option");
            if !attrs.default && !optional {
                required.push(name.clone());
            }
            properties.insert(name, self.ty(&field.ty));
        }
        json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
    }

    fn named(&mut self, name: &str) {
        if self.definitions.contains_key(name) {
            return;
        }
        self.definitions.insert(name.to_owned(), Value::Null);
        let item = self
            .items
            .get(name)
            .unwrap_or_else(|| panic!("unknown CAD schema type {name}"))
            .clone();
        let value = match item {
            Item::Struct(item) => {
                assert!(attributes(&item.attrs).deny_unknown);
                self.fields(&item.fields, None)
            }
            Item::Enum(item) => {
                let attrs = attributes(&item.attrs);
                assert!(attrs.snake);
                let variants: Vec<_> = item
                    .variants
                    .iter()
                    .map(|variant| {
                        assert!(variant.attrs.is_empty(), "unsupported variant attribute");
                        let name = snake(&variant.ident.to_string());
                        if let Some(tag) = &attrs.tag {
                            assert!(attrs.deny_unknown);
                            self.fields(&variant.fields, Some((tag, &name)))
                        } else {
                            assert!(matches!(variant.fields, Fields::Unit));
                            json!({"const":name})
                        }
                    })
                    .collect();
                json!({"oneOf": variants})
            }
            _ => unreachable!(),
        };
        self.definitions.insert(name.to_owned(), value);
    }
}

fn main() {
    let path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../ketchup-core/src/assistant_sidecar.rs");
    println!("cargo:rerun-if-changed={}", path.display());
    let source = fs::read_to_string(path).expect("read authoritative CAD wire contract");
    let ast = syn::parse_file(&source).expect("parse CAD wire contract");
    let items = ast
        .items
        .into_iter()
        .filter_map(|item| {
            let name = match &item {
                Item::Struct(s) => s.ident.to_string(),
                Item::Enum(e) => e.ident.to_string(),
                _ => return None,
            };
            Some((name, item))
        })
        .collect();
    let mut generator = Generator {
        items,
        definitions: Map::new(),
    };
    generator.named("AssistantCadEditProgram");
    let schema = json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$ref":"#/$defs/AssistantCadEditProgram", "$defs":generator.definitions,
        "description":"Complete serde wire shape generated from ketchup-core AssistantCadEditProgram. Semantic CAD bounds, references and exact admission are additionally enforced by the application planner."
    });
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("cad-program-schema.json"),
        serde_json::to_vec(&schema).unwrap(),
    )
    .unwrap();
}
