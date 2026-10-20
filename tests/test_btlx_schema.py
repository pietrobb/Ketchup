"""Every BTLx golden file validates against the official BTLx 2.3.1 XSD
(review 2026-10-15 MF-1 to MF-3), so a written processing, a thread's
Comment and UserAttributes included, is one the format describes.

The XSD and the X3D 3.3 and XML Signature schemas it includes are downloaded
once into target/btlx-schema and checked by SHA-256. The two X3D extension
schemas are empty extension points that web3d.org no longer publishes and
nothing refers to, so empty schemas stand in for them."""
import re
import urllib.request
from hashlib import sha256
from pathlib import Path

import pytest
from lxml import etree

ROOT = Path(__file__).resolve().parents[1]
FABRICATION = (ROOT / "crates/ketchup-manufacturing/src/fabrication.rs").read_text(encoding="utf-8")
GOLDENS = sorted((ROOT / "crates/ketchup-manufacturing/tests/fixtures/btlx").glob("*.btlx"))
CACHE = ROOT / "target" / "btlx-schema"

BTLX_URL = re.search(r'BTLX_2_3_1_SCHEMA_URL: &str = "([^"]+)"', FABRICATION).group(1)
BTLX_SHA256 = re.search(r'BTLX_2_3_1_SCHEMA_SHA256: &str =\s*"([0-9a-f]{64})"', FABRICATION).group(1)
X3D_URL = "https://www.web3d.org/specifications/x3d-3.3.xsd"
X3D_SHA256 = "a687b059a480a599b4bba0ca78c20ed17202ffe06450c24379833e3a26fe875b"
XMLDSIG_URL = "http://www.w3.org/TR/2002/REC-xmldsig-core-20020212/xmldsig-core-schema.xsd"
XMLDSIG_SHA256 = "35cf8197da812c85e40d57891b35c94187569ed474a2dac813ce5090dafcd35c"
EMPTY_SCHEMA = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" elementFormDefault="qualified"/>\n'
)


def fetch(url: str, digest: str) -> bytes:
    path = CACHE / url.rsplit("/", 1)[1]
    if path.exists() and sha256(path.read_bytes()).hexdigest() == digest:
        return path.read_bytes()
    request = urllib.request.Request(url, headers={"User-Agent": "ketchup-ci"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read()
    assert sha256(data).hexdigest() == digest, f"{url} changed"
    CACHE.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return data


@pytest.fixture(scope="module")
def schema() -> etree.XMLSchema:
    btlx = fetch(BTLX_URL, BTLX_SHA256).replace(X3D_URL.encode(), b"x3d-3.3.local.xsd")
    x3d = fetch(X3D_URL, X3D_SHA256).replace(XMLDSIG_URL.encode(), b"xmldsig-core-schema.xsd")
    fetch(XMLDSIG_URL, XMLDSIG_SHA256)
    for name in ("x3d-3.3-Web3dExtensionsPublic.xsd", "x3d-3.3-Web3dExtensionsPrivate.xsd"):
        (CACHE / name).write_text(EMPTY_SCHEMA, encoding="utf-8")
    (CACHE / "x3d-3.3.local.xsd").write_bytes(x3d)
    (CACHE / "BTLx_2_3_1.local.xsd").write_bytes(btlx)
    return etree.XMLSchema(etree.parse(str(CACHE / "BTLx_2_3_1.local.xsd")))


def test_there_are_goldens_and_one_carries_a_thread():
    assert len(GOLDENS) >= 10
    assert any(b'Name="ThreadSize"' in golden.read_bytes() for golden in GOLDENS)


@pytest.mark.parametrize("golden", GOLDENS, ids=lambda golden: golden.name)
def test_golden_validates_against_the_btlx_xsd(schema, golden):
    assert schema.validate(etree.parse(str(golden))), schema.error_log


def test_the_xsd_refuses_a_drilling_it_does_not_describe(schema):
    golden = next(golden for golden in GOLDENS if golden.name.startswith("tapped-drilling"))
    text = golden.read_text(encoding="utf-8").replace(
        '<Drilling Name="Ketchup tapped drilling"', '<Drilling Thread="M12" Name="Ketchup tapped drilling"', 1
    )
    assert "Thread=" in text
    assert not schema.validate(etree.fromstring(text.encode("utf-8")))
