//! The Windows code page of a DXF file older than 2007, named by its `$DWGCODEPAGE` header.

use super::DxfImportError;

/// The text of a DXF file, and the name of the code page it was read in, if any. DXF
/// 2007 and later are UTF-8; an older file is written in the Windows code page its
/// `$DWGCODEPAGE` header names, so a file that is not UTF-8 is read in that code page
/// when it is older than 2007 and names one.
pub(super) fn dxf_source_text(
    source: &[u8],
) -> Result<(std::borrow::Cow<'_, str>, Option<String>), DxfImportError> {
    if source.is_empty() {
        return Err(DxfImportError::Empty);
    }
    if let Ok(text) = std::str::from_utf8(source) {
        return Ok((std::borrow::Cow::Borrowed(text), None));
    }
    let header_value = |variable: &[u8]| {
        let mut lines = source.split(|byte| *byte == b'\n').map(<[u8]>::trim_ascii);
        lines.position(|line| line == variable)?;
        lines.nth(1)
    };
    let name = header_value(b"$ACADVER")
        .is_none_or(|version| version < b"AC1021".as_slice())
        .then(|| header_value(b"$DWGCODEPAGE"))
        .flatten()
        .ok_or(DxfImportError::NonAscii)?;
    let encoding = code_page_encoding(name).ok_or(DxfImportError::NonAscii)?;
    let text = encoding
        .decode_without_bom_handling_and_without_replacement(source)
        .ok_or(DxfImportError::NonAscii)?;
    Ok((text, Some(code_page_name(name))))
}

/// A `$DWGCODEPAGE` name [`code_page_encoding`] reads, as AutoCAD writes it: `ANSI_1250`.
pub(super) fn code_page_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// The encoding of an AutoCAD `$DWGCODEPAGE` name such as `ANSI_1250`.
pub(super) fn code_page_encoding(name: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let number = name
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case(b"ANSI_"))
        .and(name.get(5..))?;
    match number {
        b"874" | b"1250" | b"1251" | b"1252" | b"1253" | b"1254" | b"1255" | b"1256" | b"1257"
        | b"1258" => {
            let mut label = b"windows-".to_vec();
            label.extend_from_slice(number);
            encoding_rs::Encoding::for_label(&label)
        }
        b"932" => Some(encoding_rs::SHIFT_JIS),
        b"936" => Some(encoding_rs::GBK),
        b"949" => Some(encoding_rs::EUC_KR),
        b"950" => Some(encoding_rs::BIG5),
        _ => None,
    }
}
