"""Run with PYTHONPATH pointing to sdk/python and the CLI/worker built.

python sdk/python/examples/headless_model.py --executable /path/ketchup-headless
    --worker /path/ketchup-exact-worker --output model.ketchup

No GUI/LLM is started. This uses sequential atomic CAD programs, not an atomic
Python script. Any unavailable exact/validator evidence raises rather than
being described as a pass. The blind pocket is a real material-removing hole,
not a decorative cylinder or a claimed through-hole. Existing files are never
overwritten unless --overwrite is explicitly supplied.
"""
import argparse
import json

from ketchup import Session, rectangle


def only(values):
    if len(values) != 1:
        raise RuntimeError(f"Expected exactly one returned ID, got {values!r}")
    return values[0]


def created_feature(result, kinds):
    created = set(result["created"]["feature_ids"])
    return only([f["id"] for f in result["state"]["features"]
                 if f["id"] in created and f["kind"] in kinds])


def exact(document):
    report = document.evaluate()
    if not report["complete"]:
        raise RuntimeError(f"Exact evaluation incomplete: {report!r}")
    return report


def gravity(document):
    report = document.validators.run("gravity_support")
    # The public CLI returns the application's honest existing report verbatim.
    # Locate its named gravity entry without interpreting skipped as success.
    def locate(value):
        if isinstance(value, dict):
            if "gravity_support" in value and isinstance(value["gravity_support"], dict):
                return value["gravity_support"]
            for child in value.values():
                found = locate(child)
                if found is not None:
                    return found
        return None
    entry = locate(report)
    if entry is None or not entry.get("complete"):
        raise RuntimeError(f"Gravity support was not completely evaluated: {report!r}")
    return entry


def build(session, output, *, overwrite=False):
    document = session.new_document()
    base = document.box("Base with blind hole", 100, 80, 20)
    definition = only(base["created"]["definition_ids"])
    base_occurrence = only(base["created"]["occurrence_ids"])
    pad = created_feature(base, {"Pad", "Extrusion"})
    # The hole is at one end; the second body's support footprint avoids it.
    sketch = document.create_sketch(definition, "Hole profile",
                                    rectangle(10, 10, origin_mm=(10, 10)))
    profile = created_feature(sketch, {"Sketch", "Profile"})
    document.pocket(definition, "Blind hole cut", pad, profile, 10)
    document.set_grounded([base_occurrence])
    floating = document.box("Floating body", 20, 20, 10, translation_mm=(50, 30, 50))
    occurrence = only(floating["created"]["occurrence_ids"])
    exact(document)
    rejected = gravity(document)
    if rejected.get("state") not in {"failed", "rejected", "fail"}:
        raise RuntimeError(f"Expected unsupported floating body, got {rejected!r}")
    document.move([occurrence], [0, 0, -30])
    evidence = exact(document)
    accepted = gravity(document)
    if accepted.get("state") not in {"passed", "accepted", "pass"}:
        raise RuntimeError(f"Expected supported body after move, got {accepted!r}")
    saved = document.save(output, overwrite=overwrite)
    return {"state": saved["state"], "evaluation": evidence,
            "gravity_before": rejected, "gravity_after": accepted}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable")
    parser.add_argument("--worker")
    parser.add_argument("--output", required=True)
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()
    with Session(executable=args.executable, worker=args.worker) as session:
        proof = build(session, args.output, overwrite=args.overwrite)
    print(json.dumps(proof, indent=2, allow_nan=False))
    # A separate integration test should reopen in a FRESH Session and compare
    # canonical_digest, producer IDs/native geometry fingerprints and findings.


if __name__ == "__main__":
    main()
