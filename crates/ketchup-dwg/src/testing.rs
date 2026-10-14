//! Damaged DWG files for tests of the readers' limits.

use acadrust::io::dwg::checksum::apply_magic_sequence;

/// Offset and length of the AC18 inner file header, XOR-masked with the magic sequence.
const INNER_HEADER: std::ops::Range<usize> = 0x80..0x80 + 0x6C;
/// Inner header field: page map address minus 0x100.
const PAGE_MAP_ADDRESS: usize = 0x54;
/// Inner header field: page number of the section map.
const SECTION_MAP_ID: usize = 0x5C;
/// The page number the forged section map gets; the writer numbers far fewer pages.
const FORGED_SECTION_MAP_ID: i32 = 0x7000;

/// `dwg`, an AC1018-or-later file such as [`crate::dxf_to_dwg`] writes, with a section map
/// that declares its `AcDb:Header` section `declared_bytes` long. A reader that allocates the
/// declared size before reading the section asks for that much memory at once.
///
/// The forged section map and a page map that finds it are appended uncompressed, and the
/// inner file header points at them; the original pages stay where they were.
///
/// # Panics
/// `dwg` is not a page-based DWG file.
#[must_use]
pub fn with_declared_header_section(dwg: &[u8], declared_bytes: u64) -> Vec<u8> {
    let mut file = dwg.to_vec();
    let mut inner = file[INNER_HEADER].to_vec();
    apply_magic_sequence(&mut inner);
    assert_eq!(&inner[..12], b"AcFssFcAJMB\0", "a page-based DWG file");

    let section_map = system_page(&forged_section_map(declared_bytes));
    let section_map_at = file.len();
    file.extend_from_slice(&section_map);

    // Pages are found by adding up page sizes from 0x100: one entry without a number covers
    // everything before the forged section map.
    let mut page_map_data = Vec::new();
    for (number, size) in [
        (0_i32, section_map_at - 0x100),
        (FORGED_SECTION_MAP_ID, section_map.len()),
    ] {
        page_map_data.extend_from_slice(&number.to_le_bytes());
        page_map_data.extend_from_slice(&i32::try_from(size).expect("a small file").to_le_bytes());
    }
    let page_map_at = file.len();
    file.extend_from_slice(&system_page(&page_map_data));

    let stored_address = u64::try_from(page_map_at - 0x100).expect("a small file");
    inner[PAGE_MAP_ADDRESS..PAGE_MAP_ADDRESS + 8].copy_from_slice(&stored_address.to_le_bytes());
    inner[SECTION_MAP_ID..SECTION_MAP_ID + 4].copy_from_slice(&FORGED_SECTION_MAP_ID.to_le_bytes());
    apply_magic_sequence(&mut inner);
    file[INNER_HEADER].copy_from_slice(&inner);
    file
}

/// A section map with one descriptor: `AcDb:Header`, `declared_bytes` long, without pages.
fn forged_section_map(declared_bytes: u64) -> Vec<u8> {
    let mut map = Vec::new();
    for field in [1_i32, 2, 0x7400, 0, 1] {
        map.extend_from_slice(&field.to_le_bytes());
    }
    map.extend_from_slice(&declared_bytes.to_le_bytes());
    // Page count, largest page, unknown, compressed (2), section id, encrypted.
    for field in [0_i32, 0x7400, 1, 2, 1, 0] {
        map.extend_from_slice(&field.to_le_bytes());
    }
    let mut name = [0_u8; 64];
    name[..11].copy_from_slice(b"AcDb:Header");
    map.extend_from_slice(&name);
    map
}

/// An uncompressed system page: type, sizes, compression 1 and a checksum readers skip.
fn system_page(data: &[u8]) -> Vec<u8> {
    let size = i32::try_from(data.len()).expect("a small page");
    let mut page = Vec::new();
    for field in [0x4163_0003_i32, size, size, 1, 0] {
        page.extend_from_slice(&field.to_le_bytes());
    }
    page.extend_from_slice(data);
    page
}
