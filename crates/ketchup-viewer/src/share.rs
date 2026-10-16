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

/// Largest file the Viewer takes from a link or a picked file: a package, or
/// a comments file when `comments`. Anything longer cannot be read anyway.
pub fn file_limit(comments: bool) -> usize {
    if comments {
        ketchup_view_format::MAX_COMMENTS_BYTES
    } else {
        ketchup_view_format::MAX_PACKAGE_FILE_BYTES
    }
}

/// A picked file of `size` bytes is refused before the browser reads it into
/// memory when it is longer than `limit`.
pub fn check_file_size(name: &str, size: f64, limit: usize) -> Result<(), Rejection> {
    // `size` is the browser's integer byte count; every limit fits u32.
    if size <= f64::from(u32::try_from(limit).unwrap_or(u32::MAX)) {
        Ok(())
    } else {
        Err(too_large("viewer.web.too_large", name, limit))
    }
}

/// The bytes of a shared model as they arrive, refused as soon as the server
/// announces or sends more than `limit`, so a huge or endless answer never
/// fills the page's memory.
pub struct Download {
    address: String,
    limit: usize,
    bytes: Vec<u8>,
}

impl Download {
    /// Starts a download whose answer declares `content_length`, if it does.
    pub fn new(
        address: &str,
        content_length: Option<&str>,
        limit: usize,
    ) -> Result<Self, Rejection> {
        let declared = content_length
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|declared| usize::try_from(declared).unwrap_or(usize::MAX));
        if declared.is_some_and(|declared| declared > limit) {
            return Err(too_large("viewer.link.too_large", address, limit));
        }
        Ok(Self {
            address: address.to_owned(),
            limit,
            // A declared length only sizes the buffer; the bytes still count.
            bytes: Vec::with_capacity(declared.unwrap_or(0)),
        })
    }

    /// Appends the next chunk, or refuses it when the model would pass the limit.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), Rejection> {
        if chunk.len() > self.limit - self.bytes.len() {
            return Err(too_large(
                "viewer.link.too_large",
                &self.address,
                self.limit,
            ));
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

fn too_large(code: &'static str, target: &str, limit: usize) -> Rejection {
    Rejection::new(code, RejectionPhase::Request)
        .target(target.to_owned())
        .reason(format!(
            "The file is larger than the {} MiB a Viewer file can have.",
            limit / (1024 * 1024)
        ))
        .fix_hint("Open the package exported from Kečup; ask the manufacturer for a new link if it was replaced.")
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

    #[test]
    fn downloads_stop_at_the_largest_file_a_viewer_reads() {
        let address = "designs/a.ketchup-view";
        // An announced length past the limit is refused before any byte.
        let announced = Download::new(address, Some("1025"), 1024).err().unwrap();
        assert_eq!(announced.code(), "viewer.link.too_large");
        assert!(Download::new(address, Some(" 18446744073709551615 "), 1024).is_err());
        // Without (or with a lying) length the bytes themselves count.
        for declared in [None, Some("10"), Some("chunked")] {
            let mut download = Download::new(address, declared, 1024).unwrap();
            download.push(&[1; 1000]).unwrap();
            download.push(&[2; 24]).unwrap();
            let past = download.push(&[3]).unwrap_err();
            assert_eq!(past.code(), "viewer.link.too_large");
            assert_eq!(download.finish().len(), 1024);
        }
        assert!(check_file_size("a.ketchup-view", 1024.0, 1024).is_ok());
        assert_eq!(
            check_file_size("a.ketchup-view", 1025.0, 1024)
                .unwrap_err()
                .code(),
            "viewer.web.too_large"
        );
        // A full package with a full comments section still fits.
        assert!(file_limit(false) > ketchup_view_format::MAX_GLB_BYTES);
        assert_eq!(file_limit(true), ketchup_view_format::MAX_COMMENTS_BYTES);
    }
}
