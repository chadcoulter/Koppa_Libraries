"""Koppa function tokens: deferred digests that break self-hash circularity.

The problem
-----------
Embedding ``sha256(package)`` inside the package changes the bytes being hashed,
so the recorded hash can never match the shipped artifact (a self-reference loop).

The Koppa move
--------------
Koppa's primitive is an UNRESOLVED relational pair: compose ratios as (a, b),
never divide until consumption.  A FunctionToken applies the same discipline to
a digest: the token is a deferred pair ``(value, unit)`` whose resolution is the
digest, but whose SERIALIZED form is a constant opaque handle.  Hashing the
package therefore hashes the handle (a fixed point), while verification resolves
the pair to compare digests.  Only the REMAINDER -- the digest value -- is traced
as a hashing value; the hashed payload carries the handle, never the digest.

Guarantees
----------
* ``token.bytes_for_hashing()`` is constant across all digest values -> no loop.
* ``token.resolve()`` recovers the digest for verification.
* ``verify(...)`` checks the resolved digest against the artifact WITHOUT the
  digest ever appearing in the artifact's hashed bytes.
"""
from __future__ import annotations

import hashlib
from dataclasses import dataclass

HANDLE = b"KOPPA-FUNCTION-TOKEN-v1"          # constant hashed placeholder
TOKEN_PREFIX = "koppa-ft1:"                  # textual serialization prefix


@dataclass(frozen=True)
class FunctionToken:
    """Deferred digest as an unresolved (value, unit) pair.

    ``value`` is the digest integer (the 'remainder'); ``unit`` is its bit width,
    so the pair represents value * 2**(-unit) in Koppa's mantissa/exponent style.
    Resolution returns the canonical lowercase hex digest string.
    """

    value: int
    unit: int = 256

    @classmethod
    def from_digest(cls, digest: bytes, unit: int | None = None) -> "FunctionToken":
        if not digest:
            raise ValueError("digest must be non-empty")
        bits = len(digest) * 8
        return cls(int.from_bytes(digest, "big"), unit or bits)

    def resolve(self) -> bytes:
        """Resolve the deferred pair to the digest bytes (on demand, once)."""
        if self.unit % 8 or self.unit <= 0:
            raise ValueError("unit must be a positive multiple of 8")
        width = self.unit // 8
        out = self.value.to_bytes(width, "big")
        if len(out) != width:
            raise ValueError("digest value does not fit declared unit")
        return out

    def bytes_for_hashing(self) -> bytes:
        """Constant opaque handle -- this is what enters the hashed payload."""
        return HANDLE

    def serialize(self) -> str:
        """Stable textual form for a manifest (handle + resolved digest)."""
        return f"{TOKEN_PREFIX}{self.resolve().hex()}"

    @classmethod
    def parse(cls, text: str) -> "FunctionToken":
        if not text.startswith(TOKEN_PREFIX):
            raise ValueError(f"not a FunctionToken: {text!r}")
        digest = bytes.fromhex(text[len(TOKEN_PREFIX):])
        return cls.from_digest(digest)


def tokenized_payload(payload: bytes, token: FunctionToken) -> bytes:
    """Append the token's OPAQUE HANDLE to the payload, not its digest.

    The resulting bytes are stable under token resolution: changing the digest
    changes the token's pair but NOT the hashed bytes.
    """
    return payload + b"\n" + token.bytes_for_hashing() + b"\n"


def seal(payload: bytes) -> tuple[bytes, FunctionToken, bytes]:
    """Produce (hashed_bytes, deferred_token, digest).

    1. hash the raw payload; 2. wrap the digest in a deferred FunctionToken;
    3. build the tokenized artifact whose hashable form uses the constant handle.
    The digest is traced ONLY as the token's deferred remainder.
    """
    digest = hashlib.sha256(payload).digest()
    token = FunctionToken.from_digest(digest)
    artifact = tokenized_payload(payload, token)
    return artifact, token, digest


def verify(artifact: bytes, token: FunctionToken) -> bool:
    """Check that artifact's handle-framed payload hashes to token's digest."""
    frame = b"\n" + HANDLE + b"\n"
    if not artifact.endswith(frame):
        return False
    payload = artifact[: -len(frame)]
    return hashlib.sha256(payload).digest() == token.resolve()


def fixed_point_digest(payload: bytes, token: FunctionToken) -> bytes:
    """The hash of the artifact as it is actually shipped (handle-framed)."""
    return hashlib.sha256(tokenized_payload(payload, token)).digest()
