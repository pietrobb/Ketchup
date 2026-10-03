//! Documentation of the Starlark program library, built into the executable.
//!
//! The library (`ketchup_program::PRELUDE`) is split into topics by
//! `#@topic id: title` lines; the index lists each topic with its helpers.
use crate::tools::ToolError;
use serde_json::{Value, json};

// `EXAMPLES`: (file name, text) of every `examples/programs/*.star`, from build.rs.
include!(concat!(env!("OUT_DIR"), "/examples.rs"));

struct Topic<'a> {
    id: &'a str,
    title: &'a str,
    text: String,
}

/// The index without `name`; one topic or example with it.
pub fn library_detail(name: Option<&str>, detail: &str) -> Result<Value, ToolError> {
    if !matches!(detail, "concise" | "implementation") {
        return Err(ToolError::new(
            "invalid_arguments",
            "Docs detail must be concise or implementation.",
        ));
    }
    let (intro, topics) = topics(ketchup_program::PRELUDE);
    let Some(name) = name.filter(|name| !name.is_empty()) else {
        return Ok(json!({
            "language": "Starlark (Python subset)",
            "intro": intro,
            "topics": topics.iter().map(|topic| json!({
                "id": topic.id,
                "title": topic.title,
                "helpers": helpers(&topic.text),
            })).collect::<Vec<_>>(),
            "examples": EXAMPLES.iter().map(|(file, _)| *file).collect::<Vec<_>>(),
            "next": "program action=docs name=<topic> returns signatures, rules and an example; detail=implementation explicitly returns code. Examples return runnable source.",
        }));
    };
    if let Some(topic) = topics.iter().find(|topic| topic.id == name) {
        let text = if detail == "implementation" {
            topic.text.clone()
        } else {
            concise(&topic.text)
        };
        return Ok(
            json!({"topic": topic.id, "title": topic.title, "detail": detail, "text": text,
            "example": include_str!("../../ketchup-program/library/join-example.star"),
            "next": "Reuse this context; request detail=implementation only to inspect helper code."}),
        );
    }
    if let Some((file, text)) = EXAMPLES.iter().find(|(file, _)| *file == name) {
        return Ok(json!({"example": file, "text": text}));
    }
    let topic_ids: Vec<_> = topics.iter().map(|topic| topic.id).collect();
    let examples: Vec<_> = EXAMPLES.iter().map(|(file, _)| *file).collect();
    Err(ToolError::new(
        "unknown_name",
        format!(
            "name must be a topic id ({}) or an example ({})",
            topic_ids.join(", "),
            examples.join(", ")
        ),
    ))
}

fn concise(text: &str) -> String {
    let mut output = Vec::new();
    let mut signature = false;
    let mut docstring = false;
    let mut after_signature = false;
    for line in text.lines() {
        if line.starts_with('#') {
            output.push(
                line.strip_prefix("# ")
                    .unwrap_or(line.trim_start_matches('#'))
                    .to_owned(),
            );
        } else if signature || (line.starts_with("def ") && !line.starts_with("def _")) {
            output.push(line.to_owned());
            signature = !line.trim_end().ends_with(':');
            after_signature = !signature;
        } else if docstring {
            output.push(line.trim().trim_end_matches("\"\"\"").to_owned());
            docstring = !line.contains("\"\"\"");
        } else if after_signature && line.trim().starts_with("\"\"\"") {
            let text = line.trim().trim_start_matches("\"\"\"");
            output.push(text.trim_end_matches("\"\"\"").to_owned());
            docstring = !text.contains("\"\"\"");
            after_signature = false;
        } else if !line.trim().is_empty() {
            after_signature = false;
        }
    }
    output.join("\n")
}

#[cfg(test)]
fn library(name: Option<&str>) -> Result<Value, ToolError> {
    library_detail(name, "concise")
}

fn topics(source: &str) -> (String, Vec<Topic<'_>>) {
    let mut intro = String::new();
    let mut topics: Vec<Topic<'_>> = Vec::new();
    // A Windows checkout may carry CRLF line ends; the AI gets plain newlines.
    for line in source.lines() {
        if let Some((id, title)) = line
            .strip_prefix("#@topic ")
            .and_then(|rest| rest.split_once(": "))
        {
            topics.push(Topic {
                id,
                title: title.trim(),
                text: String::new(),
            });
            continue;
        }
        let text = match topics.last_mut() {
            Some(topic) => &mut topic.text,
            None => &mut intro,
        };
        text.push_str(line);
        text.push('\n');
    }
    (intro.trim().to_owned(), topics)
}

/// Documented (`#   name(`) and defined (`def name(`) helpers, in order, once each.
fn helpers(text: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for line in text.lines() {
        let candidate = if let Some(rest) = line.strip_prefix("def ") {
            rest
        } else if let Some(rest) = line.strip_prefix('#')
            && rest.starts_with(char::is_whitespace)
        {
            rest.trim_start()
        } else {
            continue;
        };
        let Some((name, _)) = candidate.split_once('(') else {
            continue;
        };
        // Names starting with `_` are the library's private helpers.
        let starts_lower = name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        if starts_lower
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concise_topics_keep_public_signatures_without_function_bodies() {
        let (_, topics) = topics(ketchup_program::PRELUDE);
        for topic in topics {
            let text = concise(&topic.text);
            for signature in topic
                .text
                .lines()
                .filter(|line| line.starts_with("def ") && !line.starts_with("def _"))
            {
                assert!(
                    text.lines().any(|line| line == signature),
                    "{}: {signature}",
                    topic.id
                );
            }
        }
        assert_eq!(
            concise("# rule\ndef helper(x):\n    \"\"\"Meaning.\"\"\"\n    return hidden(x)\n"),
            "rule\ndef helper(x):\nMeaning."
        );
    }

    #[test]
    fn index_lists_topics_with_helpers_and_examples() {
        let index = library(None).unwrap();
        let topics = index["topics"].as_array().unwrap();
        let basics = topics.iter().find(|topic| topic["id"] == "basics").unwrap();
        let helpers = basics["helpers"].as_array().unwrap();
        assert!(
            helpers.iter().any(|helper| helper == "board"),
            "{helpers:?}"
        );
        let joinery = topics
            .iter()
            .find(|topic| topic["id"] == "joinery")
            .unwrap();
        assert!(
            joinery["helpers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|helper| helper == "dowels")
        );
        assert_eq!(index["examples"].as_array().unwrap().len(), EXAMPLES.len());
    }

    #[test]
    fn one_topic_or_example_returns_its_text() {
        let placement = library(Some("placement")).unwrap();
        assert!(placement["text"].as_str().unwrap().contains("def "));
        let implementation = library_detail(Some("placement"), "implementation").unwrap();
        assert!(
            placement["text"].as_str().unwrap().len()
                < implementation["text"].as_str().unwrap().len()
        );
        assert_eq!(
            helpers(placement["text"].as_str().unwrap())
                .iter()
                .filter(|name| implementation["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("def {name}(")))
                .count(),
            helpers(implementation["text"].as_str().unwrap())
                .iter()
                .filter(|name| implementation["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("def {name}(")))
                .count()
        );
        let cabinet = library(Some("cabinet.star")).unwrap();
        assert!(cabinet["text"].as_str().unwrap().contains("board("));
        let Err(unknown) = library(Some("teapot")) else {
            panic!("an unknown name is rejected");
        };
        assert!(unknown.message.contains("basics") && unknown.message.contains("cabinet.star"));
    }
}
