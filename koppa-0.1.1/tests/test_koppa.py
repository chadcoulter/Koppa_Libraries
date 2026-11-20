#!/usr/bin/env python3
"""Regression suite for the koppa package — run directly or via pytest."""
import math, random, unittest
import koppa
from koppa import SP, ProjPair, LazyRelational, DeferredChain, Ratio, compare, pair_from_ints, prefix_scan

class TestInference(unittest.TestCase):
    def test_reject_nonfinite_negative(self):
        for bad in (float('inf'), float('-inf'), float('nan'), -2.0):
            with self.assertRaises(ValueError):
                SP.from_float(bad)

    def test_subnormal(self):
        self.assertGreater(SP(1.6, -1075).to_float(), 0.0)

    def test_forward_exact(self):
        # tiny HMM vs naive float where no underflow occurs
        A = [[0.9, 0.1], [0.2, 0.8]]
        E = [[0.5, 0.5], [0.3, 0.7]]
        obs = [0, 1, 0, 1, 0]
        pi = [0.6, 0.4]
        lk, lk_mp, collapses = koppa.forward_koppa(A, E, obs, pi)
        ln, collapses_n, first_collapse = koppa.inference.forward_naive(A, E, obs, pi)
        self.assertAlmostEqual(lk, ln, places=10)
        self.assertEqual(collapses, 0)

    def test_viterbi_no_impossible_state(self):
        p = koppa.viterbi_koppa([[1,0],[0,1]], [[0.5,0],[0,0.5]], [0], [0.5,0.5])
        self.assertEqual(p, [0])

    def test_zero_provenance(self):
        d = koppa.inference.projective_demo()
        self.assertTrue(d['shared_zero_valid'])
        self.assertTrue(d['independent_zero_is_indeterminate'])
        self.assertLess(d['ratio_relerr'], 1e-12)

class TestML(unittest.TestCase):
    def test_sign_preserved(self):
        self.assertEqual(LazyRelational(1,0,-2,0).resolved_float(), -2.0)

    def test_compose(self):
        lr = LazyRelational(1,0,3,0).compose(LazyRelational(1,0,5,0))
        self.assertAlmostEqual(lr.resolved_float(), 15.0)

    def test_gauge_invariance(self):
        lr = LazyRelational(1,0,3,0)
        self.assertAlmostEqual(lr.resolved_float(), lr.rescale(7.0).resolved_float(), places=10)

    def test_inf_rejected(self):
        with self.assertRaises(ValueError):
            LazyRelational(1,0,float('inf'),0)

class TestFinance(unittest.TestCase):
    def test_compose_decimal(self):
        c = DeferredChain(); c.compose_decimal(2, 3)
        self.assertEqual(c.resolve_float(), 1.5)

    def test_decimal_matches_full(self):
        rng = random.Random(1)
        for _ in range(200):
            a = rng.randint(1, 10**12); b = rng.randint(1, 10**12)
            c1 = DeferredChain(); c1.compose_decimal(a, b)
            c2 = DeferredChain(); c2.compose(a, b)
            self.assertEqual(c1.resolve_float(), c2.resolve_float())

    def test_overflow_returns_inf(self):
        c = DeferredChain(); c.compose(1, 10**350)
        self.assertEqual(c.resolve_float(), float('inf'))

class TestPredicates(unittest.TestCase):
    def test_exact_vs_float_witness(self):
        a1 = 7952614671698527; b1 = 21159794283675467
        a2 = 5364900327947194; b2 = 14274574083889309
        self.assertEqual(float(b1/a1), float(b2/a2))
        self.assertEqual(compare(Ratio(a1,b1), Ratio(a2,b2)), -1)

    def test_bit26_no_crash(self):
        a = (1 << 25) + 1
        self.assertIn(compare(Ratio(a, a+2), Ratio(a, a+1)), (-1, 0, 1))

    def test_fuzz(self):
        rng = random.Random(9)
        for _ in range(500):
            a1 = rng.randint(1, 10**30); b1 = rng.randint(1, 10**30)
            a2 = rng.randint(1, 10**30); b2 = rng.randint(1, 10**30)
            exact = (b1*a2 > b2*a1) - (b1*a2 < b2*a1)
            self.assertEqual(compare(Ratio(a1,b1), Ratio(a2,b2)), exact)

class TestScan(unittest.TestCase):
    def test_matches_fold(self):
        from koppa.scan import compose, resolve
        rng = random.Random(3)
        seq = [pair_from_ints(rng.randint(1,1000), rng.randint(1,1000)) for _ in range(100)]
        out = prefix_scan(seq)
        acc = seq[0]
        # scan vs fold differ only by floating reassociation (~1e-14 rel);
        # compare with relative tolerance, as the verification campaign did.
        def close(x, y):
            return abs(x-y) <= 1e-12 * max(abs(x), abs(y), 1.0)
        self.assertTrue(close(resolve(out[0]), resolve(acc)))
        for i in range(1, len(seq)):
            acc = compose(acc, seq[i])
            self.assertTrue(close(resolve(out[i]), resolve(acc)))

    def test_huge_int_rejected(self):
        with self.assertRaises(ValueError):
            pair_from_ints(1, 10**400)

if __name__ == '__main__':
    unittest.main(verbosity=2)
