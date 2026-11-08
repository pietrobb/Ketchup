//! A menu path the documents name, as **File → Import → DXF/DWG as
//! underlay…** in the README or `Subor > Vykresy` in START_HERE, is walked in
//! the real menus of the window, so a path to an item that moved or was
//! renamed fails here instead of sending a user to look for it (review
//! 2026-11-08 DOC-251).

use super::*;
use ketchup_interaction::LocaleCatalog;

const MENUS: [&str; 9] = [
    "menu-file",
    "menu-edit",
    "menu-view",
    "menu-draw",
    "menu-tools",
    "menu-model",
    "menu-window",
    "menu-plugins",
    "menu-help",
];
const SEPARATORS: [&str; 3] = [" > ", " → ", " ▸ "];

/// One space for each run of white space and `...` as `…`.
fn simplify(text: &str) -> Vec<char> {
    let mut simple: Vec<char> = Vec::with_capacity(text.len());
    for c in text.replace("...", "…").chars() {
        if !c.is_whitespace() {
            simple.push(c);
        } else if simple.last() != Some(&' ') {
            simple.push(' ');
        }
    }
    simple
}

/// Lowercase and without diacritics, as START_HERE writes Slovak.
fn fold_char(c: char) -> char {
    const FROM: &str = "áäčďéěíĺľňóôŕřšťúůýž";
    const TO: &str = "aacdeeillnoorrstuuyz";
    let c = c.to_lowercase().next().unwrap_or(c);
    FROM.chars()
        .position(|d| d == c)
        .and_then(|at| TO.chars().nth(at))
        .unwrap_or(c)
}

/// Simplified and folded, without Markdown emphasis or code marks.
fn fold(text: &str) -> String {
    simplify(text)
        .into_iter()
        .filter(|c| !matches!(c, '*' | '`'))
        .map(fold_char)
        .collect()
}

/// The documents a user reads, with the section of CHANGELOG still to be
/// released; the released sections tell the menus as they were.
fn documents() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    let mut folders = vec![root.join("docs"), root.join("scripts"), root.join("viewer")];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(name.as_str(), "archive" | "node_modules" | "target") {
                    folders.push(path);
                }
            } else if [".md", ".ps1", ".sh", ".txt"]
                .iter()
                .any(|e| name.ends_with(e))
            {
                found.push(path);
            }
        }
    }
    found.push(root.join("README.md"));
    let mut documents: Vec<(PathBuf, String)> = found
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            (path, text)
        })
        .collect();
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
    let unreleased = changelog
        .split_once("## Unreleased")
        .map(|(_, after)| after.split("\n## ").next().unwrap_or(""))
        .unwrap_or("");
    documents.push((root.join("CHANGELOG.md"), unreleased.to_owned()));
    documents
}

/// A menu path a document names: the menu, what follows its first separator
/// (folded), and whether that is the whole path, as one set in `**…**` or
/// `*…*` is; a path in plain text runs on into the sentence.
#[derive(PartialEq)]
struct MenuPath {
    menu: &'static str,
    rest: String,
    whole: bool,
}

/// The menu paths of `text`: a menu's name, capitalized as on the menu bar
/// and not ending a word before it (`New model →` names no menu), then a
/// separator.
fn paths(text: &str, catalogs: &[LocaleCatalog]) -> Vec<MenuPath> {
    let text = simplify(text);
    let mut paths = Vec::new();
    for catalog in catalogs {
        for menu in MENUS {
            for separator in SEPARATORS {
                let start: Vec<char> = format!("{}{separator}", catalog.text(menu))
                    .chars()
                    .map(fold_char)
                    .collect();
                for at in 0..(text.len() + 1).saturating_sub(start.len()) {
                    let named = text[at].is_uppercase()
                        && text[at..at + start.len()]
                            .iter()
                            .zip(&start)
                            .all(|(&c, &s)| fold_char(c) == s);
                    let before = at.checked_sub(1).map(|b| text[b]);
                    if !named || before.is_some_and(char::is_alphanumeric) {
                        continue;
                    }
                    let whole = before == Some('*');
                    let after = text[at + start.len()..].iter();
                    let rest: String = if whole {
                        after.take_while(|&&c| c != '*').collect()
                    } else {
                        after.take(200).collect()
                    };
                    let path = MenuPath {
                        menu,
                        rest: fold(&rest),
                        whole,
                    };
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
    }
    paths
}

/// The names an item may go by in a document: the whole name, and the name
/// up to its first comma or parenthesis, as "Largest drill" for the slider
/// "Largest drill, mm (0: …)".
fn names(item: &str) -> Vec<String> {
    let whole = fold(item).trim_end_matches('…').trim().to_owned();
    let mut names = vec![whole.clone()];
    if let Some(cut) = whole.find([',', '(']) {
        names.push(whole[..cut].trim().to_owned());
    }
    names.retain(|name| !name.is_empty());
    names
}

/// Walk `path` from its menu: each name up to a separator must be a submenu
/// of the one opened before, and the last one an item in it. A whole path
/// must end with that name; one in plain text may run on into the sentence,
/// but not past a submenu into a name the submenu does not offer, as
/// `Súbor > Importovať ohraničenú 2D podmnožinu` does.
fn walk(shell: &mut Shell, path: &MenuPath) -> Result<(), String> {
    let mut before = shell.menu_items();
    shell.open_menu(path.menu);
    let mut walked = shell.catalog().text(path.menu);
    let mut rest = path.rest.as_str();
    loop {
        // Only what the menu just opened offers, not a button of the same
        // name elsewhere in the window.
        let shown = shell.menu_items();
        let items: Vec<_> = shown.iter().filter(|item| !before.contains(item)).collect();
        let found = items
            .iter()
            .flat_map(|(item, submenu, _)| names(item).into_iter().map(move |n| (n, submenu)))
            .filter(|(name, _)| {
                rest.strip_prefix(name.as_str()).is_some_and(|after| {
                    !after
                        .trim_start_matches('…')
                        .starts_with(|c: char| c.is_alphanumeric() || c == '/')
                })
            })
            .max_by_key(|(name, submenu)| (name.len(), submenu.is_some()));
        let quoted: String = rest.chars().take(60).collect();
        let Some((name, submenu)) = found else {
            return Err(format!("{walked} offers nothing named {quoted:?}"));
        };
        let after = rest[name.len()..].trim_start_matches('…');
        let Some(next) = SEPARATORS.iter().find_map(|s| after.strip_prefix(s)) else {
            let runs_on = if path.whole {
                !after.trim().is_empty()
            } else {
                submenu.is_some()
                    && after.starts_with(' ')
                    && after[1..].starts_with(char::is_alphanumeric)
            };
            return if runs_on {
                Err(format!("{walked} offers no item named {quoted:?}"))
            } else {
                Ok(())
            };
        };
        let Some(submenu) = submenu.clone() else {
            return Err(format!("{walked} > {name} is not a submenu"));
        };
        before = shown;
        shell.open_submenu_labelled(&submenu);
        walked = format!("{walked} > {}", submenu.trim_end_matches(" ⏵"));
        rest = next;
    }
}

#[test]
fn every_menu_path_a_document_names_is_offered_by_the_menus() {
    let catalogs = [LocaleCatalog::english(), LocaleCatalog::slovak()];
    let mut shells = [
        Shell::with_catalog(LocaleCatalog::english()),
        Shell::with_catalog(LocaleCatalog::slovak()),
    ];
    let mut wrong = Vec::new();
    let mut walked = 0;
    for (path, text) in documents() {
        for menu_path in paths(&text, &catalogs) {
            walked += 1;
            let mut errors = Vec::new();
            for shell in &mut shells {
                let result = walk(shell, &menu_path);
                for _ in 0..3 {
                    shell.press_key(Key::Escape);
                }
                match result {
                    Ok(()) => {
                        errors.clear();
                        break;
                    }
                    Err(error) => errors.push(error),
                }
            }
            if !errors.is_empty() {
                wrong.push(format!("{}: {}", path.display(), errors.join(" | ")));
            }
        }
    }
    assert!(walked >= 10, "only {walked} menu paths were found");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
