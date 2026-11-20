#!/usr/bin/env python3
"""Tests for FunktionToken / self-hash-breaking function tokens."""
import hashlib, unittest
import koppa
from koppa import (FunctionToken, tokenized_payload, seal, verify,
                   fixed_point_digest, HANDLE)


class TestFunctionToken(unittest.TestCase):
    def test_handle_is_constant_across_digests(self):
        a = FunctionToken.from_digest(hashlib.sha256(b"a").digest())
        b = FunctionToken.from_digest(hashlib.sha256(b"b").digest())
        self.assertNotEqual(a.value, b.value)                      # digests differ
        self.assertEqual(a.bytes_for_hashing(), b.bytes_for_hashing())  # handles same
        self.assertEqual(a.bytes_for_hashing(), HANDLE)

    def test_seal_and_verify(self):
        payload = b"iching_photonic_corpus_v6"
        artifact, token, digest = seal(payload)
        self.assertTrue(verify(artifact, token))
        self.assertEqual(token.resolve(), digest)

    def test_digest_not_in_hashed_bytes(self):
        payload = b"package-bytes"
        artifact, token, digest = seal(payload)
        self.assertNotIn(digest.hex().encode(), artifact)
        self.assertIn(HANDLE, artifact)

    def test_fixed_point_is_stable(self):
        payload = b"stable"
        artifact, token, digest = seal(payload)
        fp1 = fixed_point_digest(payload, token)
        fp2 = hashlib.sha256(artifact).digest()
        self.assertEqual(fp1, fp2)
        self.assertNotEqual(fp1, digest)  # frame changes the hash

    def test_negative_control_tampered_artifact(self):
        payload = b"original"
        artifact, token, digest = seal(payload)
        tampered = artifact.replace(b"original", b"tampered")
        self.assertFalse(verify(tampered, token))

    def test_negative_control_wrong_token(self):
        artifact, token, digest = seal(b"data")
        wrong = FunctionToken.from_digest(hashlib.sha256(b"other").digest())
        self.assertFalse(verify(artifact, wrong))

    def test_negative_control_bad_frame(self):
        artifact, token, digest = seal(b"data")
        self.assertFalse(verify(b"no frame here", token))

    def test_serialize_parse_roundtrip(self):
        token = FunctionToken.from_digest(hashlib.sha256(b"rt").digest())
        self.assertEqual(FunctionToken.parse(token.serialize()), token)

    def test_empty_digest_rejected(self):
        with self.assertRaises(ValueError):
            FunctionToken.from_digest(b"")

    def test_bad_unit_rejected(self):
        with self.assertRaises(ValueError):
            FunctionToken(1, unit=7).resolve()


if __name__ == "__main__":
    unittest.main()
