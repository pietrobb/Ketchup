//! Shared Viewer links: `viewer/?model=<address>` opens one exported design.
//! Only the hosting page's own files, or origins its owner lists, are opened,
//! so a crafted link cannot show a foreign model under the host's address.

use ketchup_rejection::{Rejection, RejectionPhase};

/// The address of the model a shared link opens, from the page query
/// (`?model=...`); `None` when the link names no model. Relative addresses
/// stay on the hosting page's origin; absolute ones must be `http(s)` on one
/// of `allowed_origins` (e.g. `https://cdn.example.com`).
pub fn shared_model(query: &str, allowed_origins: &[&str]) -> Result<Option<String>, Rejection> {
    let Some(value) = query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('=').or(Some((pair, ""))))
        .find(|(key, _)| *key == "model")
        .map(|(_, value)| value)
    else {
        return Ok(None);
    };
    let address = percent_decode(value).ok_or_else(|| link_error(value))?;
    // A browser strips spaces around an address before resolving it, so
    // ` //evil.example/x` would leave the host; it is refused, not classified.
    if address.is_empty()
        || address.starts_with(' ')
        || address.ends_with(' ')
        || address.chars().any(|c| c.is_control() || c == '\\')
    {
        return Err(link_error(&address));
    }
    if address.starts_with("//") {
        return allowed(
            &address,
            origin_of(&format!("https:{address}")),
            allowed_origins,
        );
    }
    match address.split_once(':') {
        Some((scheme, _)) if !scheme.contains('/') => {
            let origin = ["http", "https"]
                .iter()
                .any(|s| scheme.eq_ignore_ascii_case(s))
                .then(|| origin_of(&address))
                .flatten();
            allowed(&address, origin, allowed_origins)
        }
        _ => Ok(Some(address)),
    }
}

/// The name a download from the Viewer is saved under: the model's file name
/// without `.ketchup-view`, followed by `suffix` (e.g. `.ketchup-comments`).
pub fn download_name(model_address: Option<&str>, suffix: &str) -> String {
    let name = model_address
        .and_then(|address| address.split(['?', '#']).next())
        .and_then(|address| address.rsplit('/').next())
        .map(|name| name.strip_suffix(".ketchup-view").unwrap_or(name))
        .filter(|name| !name.is_empty() && name.chars().all(|c| !c.is_control()))
        .unwrap_or("model");
    format!("{name}{suffix}")
}

fn allowed(
    address: &str,
    origin: Option<String>,
    allowed_origins: &[&str],
) -> Result<Option<String>, Rejection> {
    let origin = origin.ok_or_else(|| link_error(address))?;
    if allowed_origins
        .iter()
        .any(|allowed| allowed.trim_end_matches('/').eq_ignore_ascii_case(&origin))
    {
        Ok(Some(address.to_owned()))
    } else {
        Err(
            Rejection::new("viewer.link.origin", RejectionPhase::Request)
                .target(origin)
                .reason("This Viewer does not open models from that address.")
                .fix_hint("Open the link published by the manufacturer, or ask them to allow the model's origin."),
        )
    }
}

/// `scheme://host[:port]`, lower-cased; `None` without a host.
fn origin_of(address: &str) -> Option<String> {
    let (scheme, rest) = address.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{authority}").to_ascii_lowercase())
}

fn percent_decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let digits = [input.next()?, input.next()?];
                u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok()?
            }
            other => other,
        });
    }
    String::from_utf8(bytes).ok()
}

fn link_error(address: &str) -> Rejection {
    Rejection::new("viewer.link.invalid", RejectionPhase::Request)
        .target(address.to_owned())
        .reason("The shared link does not name a model this Viewer can open.")
        .fix_hint("Copy the whole link again from the manufacturer's page.")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CDN: &[&str] = &["https://cdn.example.com/"];

    #[test]
    fn relative_model_stays_on_the_hosting_origin() {
        assert_eq!(
            shared_model("?model=designs%2Fkuchy%C5%88a+2.ketchup-view", &[]).unwrap(),
            Some("designs/kuchyňa 2.ketchup-view".to_owned())
        );
        assert_eq!(shared_model("?lang=sk", &[]).unwrap(), None);
        assert_eq!(shared_model("", &[]).unwrap(), None);
    }

    #[test]
    fn foreign_origins_need_the_hosts_permission() {
        let cdn = "https://CDN.example.com/a.ketchup-view";
        assert_eq!(
            shared_model(&format!("model={cdn}"), CDN)
                .unwrap()
                .as_deref(),
            Some(cdn)
        );
        for link in [
            "model=https://evil.example/a.ketchup-view",
            "model=//evil.example/a.ketchup-view",
            "model=https://cdn.example.com.evil.example/a.ketchup-view",
            "model=https://cdn.example.com@evil.example/a.ketchup-view",
            // The browser strips the space and leaves the host (SEC-4).
            "model=+//evil.example/a.ketchup-view",
            "model=%20%20https://evil.example/a.ketchup-view",
            "model=//evil.example/a.ketchup-view+",
        ] {
            let code = shared_model(link, CDN).unwrap_err().code().to_owned();
            assert!(code.starts_with("viewer.link."), "{link}: {code}");
        }
    }

    #[test]
    fn script_data_and_malformed_links_are_refused() {
        for link in [
            "model=javascript:alert(1)",
            "model=data:application/octet-stream,AAAA",
            "model=file:///C:/a.ketchup-view",
            "model=",
            "model=%E0%A4",
            "model=%zz",
            "model=a%0Ab",
            "model=..%5Csecret",
        ] {
            let code = shared_model(link, CDN).unwrap_err().code().to_owned();
            assert!(code.starts_with("viewer.link."), "{link}: {code}");
        }
    }

    #[test]
    fn comments_download_under_the_models_name() {
        let comments = ".ketchup-comments";
        assert_eq!(
            download_name(Some("designs/kuchyňa.ketchup-view?v=3"), comments),
            "kuchyňa.ketchup-comments"
        );
        assert_eq!(
            download_name(Some("kuchyňa.ketchup-view"), "-comments.ketchup-view"),
            "kuchyňa-comments.ketchup-view"
        );
        assert_eq!(
            download_name(Some("designs/"), comments),
            "model.ketchup-comments"
        );
        assert_eq!(download_name(None, comments), "model.ketchup-comments");
    }
}
