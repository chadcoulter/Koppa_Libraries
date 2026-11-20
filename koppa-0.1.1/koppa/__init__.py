"""koppa — deferred relational tracking.

Keep a ratio unresolved as a pair (a, b); compose without dividing;
resolve to a scalar only when consumed. Verified on 600-700 decade
chains with zero collapses; exact ratio predicates; associative scans.

Not a new calculus: standard log-domain / projective-coordinate practice
given a type-level interface. See README for measured results and limits.
"""
from .inference import SP, ProjPair, sp_max, forward_koppa, viterbi_koppa
from .ml import LazyRelational
from .finance import DeferredChain
from .predicates import Ratio, compare
from .scan import pair_from_ints, scan_double_buffer as prefix_scan, scan_in_place
from .token import (FunctionToken, tokenized_payload, seal, verify,
                    fixed_point_digest, HANDLE, TOKEN_PREFIX)

__version__ = "0.1.1"
__all__ = ["SP", "ProjPair", "sp_max", "forward_koppa", "viterbi_koppa",
           "LazyRelational", "DeferredChain", "Ratio", "compare",
           "pair_from_ints", "prefix_scan", "scan_in_place",
           "FunctionToken", "tokenized_payload", "seal", "verify",
           "fixed_point_digest", "HANDLE", "TOKEN_PREFIX", "__version__"]
