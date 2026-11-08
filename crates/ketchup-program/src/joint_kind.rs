//! What a joint's `kind` says of its parts: that it fastens them, that they only
//! meet, or neither, as a closed catalogue in English and the words Slovak, Czech
//! and German programs use (review 2026-11-08 PG-241: every kind but seven English
//! words fastened, so "voľne", "volně", "kontakt" or "nichts" braced a beam unsaid).

/// What a joint's kind says of how its parts join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointFastening {
    /// A fastening the catalogue knows: a screw, a hanger, glue, a dowel.
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
    "glue",
    "glued",
    "adhesive",
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
    "mortise and tenon",
    "dovetail",
    "arunda",
    "biscuit",
    "cam",
    "cam lock",
    "hinge",
    "slide",
    "motion",
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
    "lepidlo",
    "lepené",
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
    "lepený",
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
    "leim",
    "geleimt",
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

/// Whether `kind` (normalized) begins with the whole words of `entry`: programs
/// qualify a kind by side, size or product ("krokvová spojka ľavá", "vrut 6x120",
/// "Arunda 80 B"), and the qualifier does not change what the kind fastens.
fn begins_with(kind: &str, entry: &str) -> bool {
    kind.strip_prefix(entry)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// What `kind` says of how a joint's parts join: the catalogue entry its leading
/// words are, an unfastened one over a fastening one when both are.
#[must_use]
pub fn fastening(kind: &str) -> JointFastening {
    let kind = normalized(kind);
    if UNFASTENED_JOINT_KINDS
        .iter()
        .any(|entry| begins_with(&kind, entry))
    {
        JointFastening::Unfastened
    } else if FASTENING_JOINT_KINDS
        .iter()
        .any(|entry| begins_with(&kind, entry))
    {
        JointFastening::Fastens
    } else {
        JointFastening::Unknown
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
}
