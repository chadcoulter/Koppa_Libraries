#!/usr/bin/env python3
"""Koppa post-build artifact integrity example.

'candidate' records an UNAPPROVED build inventory.
'verify' checks bytes and Koppa tokens against an existing manifest.
A separately trusted manifest digest must be supplied for approved verification.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

from koppa import FunctionToken, seal, verify as koppa_verify, fixed_point_digest

SCHEMA = "koppa-build-integrity-v1"
HEX_SHA256 = re.compile(r"^[0-9a-f]{64}$")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode("utf-8")


def candidate(directory: Path, output: Path) -> None:
    directory = directory.resolve()
    artifacts = sorted(
        [p for p in directory.iterdir() if p.is_file() and (p.suffix == ".whl" or p.name.endswith(".tar.gz"))],
        key=lambda p: p.name,
    )
    if not artifacts:
        raise ValueError("No wheel or source distribution found")
    manifest = {"schema": SCHEMA, "approval": "UNAPPROVED_CANDIDATE", "artifacts": []}
    for path in artifacts:
        payload = path.read_bytes()
        _framed, token, digest = seal(payload)
        manifest["artifacts"].append({
            "name": path.name,
            "size": len(payload),
            "sha256": digest.hex(),
            "koppa_token": token.serialize(),
            "framed_sha256": fixed_point_digest(payload, token).hex(),
        })
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical_json(manifest))
    print("Candidate manifest:", output)
    print("Manifest SHA-256:", sha256(output.read_bytes()))
    print("Approval: UNAPPROVED_CANDIDATE")


def verify(manifest_path: Path, directory: Path, evidence_path: Path,
           expected_manifest_sha256: str | None) -> bool:
    raw = manifest_path.read_bytes()
    if expected_manifest_sha256 is not None:
        if not HEX_SHA256.fullmatch(expected_manifest_sha256):
            raise ValueError("Expected manifest digest must be 64 lowercase hex characters")
        if sha256(raw) != expected_manifest_sha256:
            raise ValueError("Manifest digest differs from independently supplied reference")
    manifest = json.loads(raw)
    if manifest.get("schema") != SCHEMA:
        raise ValueError("Unsupported manifest schema")
    entries = manifest.get("artifacts")
    if not isinstance(entries, list) or not entries:
        raise ValueError("Manifest must have at least one artifact")
    if len({entry["name"] for entry in entries}) != len(entries):
        raise ValueError("Duplicate artifact names in manifest")

    directory = directory.resolve()
    results = []
    for entry in entries:
        name = entry["name"]
        # No path traversal, absolute paths, or symlink escapes.
        if not isinstance(name, str) or Path(name).name != name or name in {".", ".."}:
            raise ValueError("Invalid artifact filename")
        path = directory / name
        result = {"name": name, "status": "FAIL", "reason": ""}
        if not path.is_file() or not path.resolve().is_relative_to(directory):
            result["reason"] = "ARTIFACT_MISSING_OR_UNSAFE_PATH"
        else:
            payload = path.read_bytes()
            result["observed_sha256"] = sha256(payload)
            try:
                token = FunctionToken.parse(entry["koppa_token"])
                expected = entry["sha256"]
                good = (
                    isinstance(expected, str)
                    and HEX_SHA256.fullmatch(expected) is not None
                    and len(payload) == entry["size"]
                    and result["observed_sha256"] == expected
                    and token.resolve().hex() == expected
                    and koppa_verify(seal(payload)[0], token)
                    and fixed_point_digest(payload, token).hex() == entry["framed_sha256"]
                )
                result["status"] = "PASS" if good else "FAIL"
                result["reason"] = "MATCH" if good else "INTEGRITY_MISMATCH"
            except (ValueError, OverflowError, KeyError, TypeError) as exc:
                result["reason"] = "INVALID_MANIFEST_OR_TOKEN"
                result["detail"] = type(exc).__name__
        results.append(result)

    trusted_reference = expected_manifest_sha256 is not None
    success = all(r["status"] == "PASS" for r in results)
    evidence = {
        "schema": SCHEMA,
        "manifest_sha256": sha256(raw),
        "reference": "EXTERNALLY_PINNED_MANIFEST" if trusted_reference else "UNPINNED_MANIFEST",
        "status": "PASS" if success else "FAIL",
        "interpretation": "APPROVED_REFERENCE_CHECK" if trusted_reference else "CANDIDATE_INTEGRITY_CHECK_ONLY",
        "artifacts": results,
    }
    evidence_path.parent.mkdir(parents=True, exist_ok=True)
    evidence_path.write_bytes(canonical_json(evidence))
    print(json.dumps(evidence, indent=2))
    return success


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    create = sub.add_parser("candidate", help="Record unapproved build outputs")
    create.add_argument("--dir", type=Path, required=True)
    create.add_argument("--output", type=Path, required=True)
    check = sub.add_parser("verify", help="Verify outputs against existing manifest")
    check.add_argument("--dir", type=Path, required=True)
    check.add_argument("--manifest", type=Path, required=True)
    check.add_argument("--evidence", type=Path, required=True)
    check.add_argument("--expected-manifest-sha256", default=None,
                       help="Independently approved manifest SHA-256 (out-of-band reference)")
    args = parser.parse_args()
    try:
        if args.command == "candidate":
            candidate(args.dir, args.output)
            return 0
        return 0 if verify(args.manifest, args.dir, args.evidence,
                           args.expected_manifest_sha256) else 1
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(f"Koppa integrity error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
