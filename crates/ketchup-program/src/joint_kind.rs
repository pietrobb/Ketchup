//! What a joint's `kind` says of its parts: that it fastens them, that they only
//! meet, or neither, as a closed catalogue in English and the words Slovak, Czech
//! and German programs use (review 2026-11-08 PG-241: every kind but seven English
//! words fastened, so "voľne", "volně", "kontakt" or "nichts" braced a beam unsaid).

/// What a joint's kind says of how its parts join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointFastening {
    /// A fastening the catalogue knows: a screw, a hanger, a structural adhesive,
    /// a dowel.
    Fastens,
    /// The parts only meet: they neither hold each other down nor brace.
    Unfastened,
    /// A kind the catalogue does not know. It is taken to fasten, as it may, and
    /// every check standing on it is not verified.
    Unknown,
}

/// Kinds that only record how two parts meet and fasten nothing (review
/// 2026-11-04 r19 PG-162, 2026-11-06 r23 P3), written as [`normalized`] writes them.
pub const UNFASTENED_JOINT_KINDS: &[&str] = &[
    // English
    "none",
    "contact",
    "friction",
    "unfastened",
    "loose",
    "free",
    "rests",
    "resting",
    "bearing only",
    // Not a structural adhesive (EN 1995-1-1 3.6: EN 301, EN 15425; R1110 P3)
    "contact adhesive",
    // Kinds of joints whose parts move (R1110-PG-405)
    "slide",
    "motion",
    // Slovak
    "žiadny",
    "žiadne",
    "ziadny",
    "nič",
    "kontakt",
    "dotyk",
    "trenie",
    "voľne",
    "volne",
    "voľný",
    "volny",
    "uložené",
    "ulozene",
    "položené",
    "polozene",
    "opreté",
    "bez spoja",
    // Czech
    "žádný",
    "žádné",
    "zadny",
    "nic",
    "tření",
    "treni",
    "volně",
    "volný",
    "uložený",
    "položený",
    "opřený",
    "bez spoje",
    // German
    "keine",
    "kein",
    "nichts",
    "berührung",
    "beruehrung",
    "reibung",
    "lose",
    "frei",
    "aufliegend",
    "aufgelegt",
];

/// Kinds that fasten their parts to each other, written as [`normalized`] writes them.
pub const FASTENING_JOINT_KINDS: &[&str] = &[
    // English
    "screw",
    "screws",
    "screwed",
    "nail",
    "nails",
    "nailed",
    "staple",
    "staples",
    "bolt",
    "bolts",
    "bolted",
    "rivet",
    "dowel",
    "dowels",
    "pin",
    "pins",
    "shelf pins",
    "structural adhesive",
    "structural glue",
    "hanger",
    "joist hanger",
    "connector",
    "bracket",
    "angle",
    "angle bracket",
    "plate",
    "nail plate",
    "strap",
    "tie",
    "rafter tie",
    "anchor",
    "hold down",
    "holddown",
    "tenon",
    "loose tenon",
    "mortise and tenon",
    "dovetail",
    "arunda",
    "biscuit",
    "cam",
    "cam lock",
    "hinge",
    "fixed",
    "clamped",
    "welded",
    // Slovak
    "vrut",
    "vruty",
    "skrutka",
    "skrutky",
    "skrutkované",
    "klinec",
    "klince",
    "klincované",
    "spona",
    "spony",
    "svorník",
    "svorníky",
    "kolík",
    "kolíky",
    "konštrukčné lepidlo",
    "spojka",
    "krokvová spojka",
    "trámová botka",
    "botka",
    "uholník",
    "kotva",
    "styčníková doska",
    "čap",
    "rybina",
    "pánt",
    // Czech
    "šroub",
    "šrouby",
    "hřebík",
    "hřebíky",
    "sponka",
    "kolíček",
    "konstrukční lepidlo",
    "úhelník",
    "styčníková deska",
    "čep",
    "rybinový spoj",
    "pant",
    // German
    "schraube",
    "schrauben",
    "nagel",
    "nägel",
    "klammer",
    "bolzen",
    "dübel",
    "duebel",
    "stabdübel",
    "konstruktionsklebstoff",
    "verbinder",
    "balkenschuh",
    "sparrenpfettenanker",
    "winkel",
    "winkelverbinder",
    "lochblech",
    "anker",
    "zapfen",
    "schwalbenschwanz",
    "scharnier",
];

/// `kind` as the catalogues write it: in lower case, its words apart by one
/// space, `_` and `-` read as spaces ("Hold_Down" is "hold down").
#[must_use]
pub fn normalized(kind: &str) -> String {
    kind.to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words that qualify a kind by its side or place, which does not change what
/// it fastens ("krokvová spojka ľavá", "Schraube links").
const SIDE_WORDS: &[&str] = &[
    "left", "right", "upper", "lower", "top", "bottom", "inner", "outer", "ľavá", "ľavý", "ľavé",
    "lavá", "lava", "pravá", "pravý", "pravé", "prava", "horná", "horný", "dolná", "dolný", "levá",
    "levý", "levé", "horní", "dolní", "links", "rechts", "oben", "unten", "innen", "außen",
    "aussen",
];

/// Whether a word is a size with a digit other than 0: numbers apart by "x" or "×" ("6x120",
/// "4,5×60", "80"), at most one letter but "x" before them ("M12", "d8") and "mm" or "cm" after
/// them ("80mm"). Other letters around a digit are no size: "screw no1" and "screw tbd1"
/// fastened (review 2026-11-13 P3).
fn sized(word: &str) -> bool {
    let word = word
        .strip_suffix("mm")
        .or_else(|| word.strip_suffix("cm"))
        .unwrap_or(word);
    let numbers = match word.chars().next() {
        Some(first) if first.is_alphabetic() && first != 'x' => &word[first.len_utf8()..],
        _ => word,
    };
    !numbers.is_empty()
        && numbers.split(['x', '×']).all(|number| {
            !number.is_empty()
                && number
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        })
        && numbers.chars().any(|c| matches!(c, '1'..='9'))
}

/// Whether `word` qualifies a kind by size, product or side: it carries a size,
/// is one letter but "x" after a size ("80 B") or names a side. A size of only
/// zeros or a lone "x" says nothing of the fastener: "screw 0", "nail x" and
/// "pin 0 x 0" fastened (R1110-PG-405), and so did any one character after the
/// kind itself: "screw y", "nail o", "screw ½" (R1112 P3).
fn qualifier(word: &str, after: &str) -> bool {
    sized(word)
        || (sized(after)
            && word.chars().count() == 1
            && word.chars().all(|c| c.is_ascii_alphabetic())
            && word != "x")
        || SIDE_WORDS.contains(&word)
}

/// The longest catalogue entry `words` begin with, and how it fastens.
fn longest_entry(words: &[&str]) -> Option<(usize, JointFastening)> {
    UNFASTENED_JOINT_KINDS
        .iter()
        .map(|entry| (entry, JointFastening::Unfastened))
        .chain(
            FASTENING_JOINT_KINDS
                .iter()
                .map(|entry| (entry, JointFastening::Fastens)),
        )
        .filter_map(|(entry, fastening)| {
            let length = entry.split(' ').count();
            (words.len() >= length && words[..length].join(" ") == **entry)
                .then_some((length, fastening))
        })
        .max_by_key(|(length, _)| *length)
}

/// What `kind` says of how a joint's parts join. Its words are read as catalogue
/// entries, the longest first ("loose tenon" fastens, "loose" alone does not),
/// and qualifiers of size, product or side ("vrut 6x120", "Arunda 80 B"). It
/// must begin with an entry, and an unfastened entry wins over a fastening one
/// ("lose Schraube"). A word that is neither leaves the kind unknown (review
/// 2026-11-09 PG-309: "slide in", "angle cut" and "plate bearing" fastened by
/// their first word alone).
#[must_use]
pub fn fastening(kind: &str) -> JointFastening {
    let kind = normalized(kind);
    let words: Vec<&str> = kind.split(' ').filter(|word| !word.is_empty()).collect();
    let mut at = 0;
    let mut found = Vec::new();
    while at < words.len() {
        if let Some((length, fastening)) = longest_entry(&words[at..]) {
            found.push(fastening);
            at += length;
        } else if at > 0 && qualifier(words[at], words[at - 1]) {
            at += 1;
        } else {
            return JointFastening::Unknown;
        }
    }
    if found.is_empty() {
        JointFastening::Unknown
    } else if found.contains(&JointFastening::Unfastened) {
        JointFastening::Unfastened
    } else {
        JointFastening::Fastens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogues_are_written_normalized_and_apart() {
        for kind in UNFASTENED_JOINT_KINDS.iter().chain(FASTENING_JOINT_KINDS) {
            assert_eq!(normalized(kind), *kind);
        }
        for kind in UNFASTENED_JOINT_KINDS {
            assert!(!FASTENING_JOINT_KINDS.contains(kind), "{kind}");
        }
    }

    /// Review 2026-11-08 PG-241.
    #[test]
    fn a_kind_fastens_only_when_the_catalogue_says_so() {
        for kind in [
            "voľne",
            "Volně",
            "kontakt",
            "dotyk",
            "nichts",
            " lose ",
            "voľne uložené",
            "lose Schraube",
        ] {
            assert_eq!(fastening(kind), JointFastening::Unfastened, "{kind}");
        }
        for kind in [
            "vrut",
            "krokvová spojka ľavá",
            "krokvová spojka pravá",
            "vrut 6x120",
            "Schraube links",
            "Arunda 80 B",
            "Hold_Down",
            "screw",
            "Schraube",
        ] {
            assert_eq!(fastening(kind), JointFastening::Fastens, "{kind}");
        }
        for kind in [
            "test",
            "contcat",
            "spoj",
            "xyz",
            "vruty2",
            "screwdriver",
            "loosely",
        ] {
            assert_eq!(fastening(kind), JointFastening::Unknown, "{kind}");
        }
    }

    /// Review 2026-11-09 PG-309: a kind was read by its first word alone.
    #[test]
    fn a_kind_is_read_by_all_its_words_not_its_first() {
        for kind in [
            "slide in",
            "angle cut",
            "plate bearing",
            "screw or not",
            "kontakt plocha",
        ] {
            assert_eq!(fastening(kind), JointFastening::Unknown, "{kind}");
        }
        for kind in [
            "loose tenon",
            "Loose_Tenon",
            "mortise and tenon",
            "angle bracket left",
            "nail plate 100x200",
        ] {
            assert_eq!(fastening(kind), JointFastening::Fastens, "{kind}");
        }
        for kind in ["loose", "contact", "lose Schraube", "voľne uložené"] {
            assert_eq!(fastening(kind), JointFastening::Unfastened, "{kind}");
        }
    }

    /// R1110-PG-405: a kind that says its parts move fastened them, and so did
    /// any word with a digit or of one letter: "screw 0", "nail x".
    #[test]
    fn a_kind_that_moves_fastens_nothing_and_a_bare_zero_qualifies_nothing() {
        for kind in ["slide", "motion", "Slide", "slide 500"] {
            assert_eq!(fastening(kind), JointFastening::Unfastened, "{kind}");
        }
        for kind in ["screw 0", "nail x", "pin 0 x 0", "vrut 000", "screw 0x0"] {
            assert_eq!(fastening(kind), JointFastening::Unknown, "{kind}");
        }
        for kind in [
            "screw 6x0",
            "Arunda 80 B",
            "vrut 6x120",
            "bolt M12",
            "dowel 8",
        ] {
            assert_eq!(fastening(kind), JointFastening::Fastens, "{kind}");
        }
    }

    /// R1112 P3: "contact adhesive" and "glue" fastened, and so did a kind with
    /// any one character after it.
    #[test]
    fn only_a_structural_adhesive_fastens_and_one_character_qualifies_only_a_size() {
        for kind in ["contact adhesive", "contact-adhesive 2", "Contact_Adhesive"] {
            assert_eq!(fastening(kind), JointFastening::Unfastened, "{kind}");
        }
        for kind in [
            "glue",
            "glued",
            "adhesive",
            "lepidlo",
            "lepené",
            "Leim",
            "screw y",
            "nail o",
            "screw ½",
            "vrut a",
            "screw 0 B",
        ] {
            assert_eq!(fastening(kind), JointFastening::Unknown, "{kind}");
        }
        for kind in [
            "structural adhesive",
            "Structural_Glue 2",
            "konštrukčné lepidlo",
            "Konstruktionsklebstoff",
            "Arunda 80 B",
            "screw 1",
        ] {
            assert_eq!(fastening(kind), JointFastening::Fastens, "{kind}");
        }
    }

    /// Review 2026-11-13 P3: any word with a digit 1–9 was a size, so "screw no1" and
    /// "screw tbd1" fastened.
    #[test]
    fn a_word_is_a_size_only_as_a_number_with_one_letter_before_it_or_a_unit_after_it() {
        for kind in [
            "screw no1",
            "screw tbd1",
            "nail a1b",
            "bolt 1st",
            "screw x1",
            "dowel 8x",
        ] {
            assert_eq!(fastening(kind), JointFastening::Unknown, "{kind}");
        }
        for kind in [
            "screw 4,5x60",
            "vrut 6×120",
            "nail 3.1x90",
            "bolt M12x1.5",
            "dowel d8",
            "screw 80mm",
        ] {
            assert_eq!(fastening(kind), JointFastening::Fastens, "{kind}");
        }
    }
}
