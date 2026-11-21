# Reusable Post-Build Integrity Verification Process

**Status:** Implementation pattern derived from the working Koppa supply-chain-integrity example  
**Applies to:** Source repositories that produce installable packages, binaries, archives, containers, or comparable build artifacts  
**Scope:** Repeatable build, artifact identity recording, and post-build verification. Release approval and authenticated supply-chain provenance are distinct, subsequent controls.

## 1. Purpose

A repository should be able to build its software, identify the exact resulting bytes, and retain machine-readable evidence that a later verification operation examined those same bytes.

This document defines a portable process that can be implemented in other repositories without copying Koppa's Python packaging assumptions. The producing language, build system, CI provider, artifact format, and integrity adapter may change; the sequence and meaning of its verification results should remain stable.

**Governing separation:**

1. **Build:** produce an artifact from a selected source revision.
2. **Record:** capture the exact artifact bytes and their identities in a *candidate manifest*.
3. **Verify:** independently read the files and compare their observed identities to a supplied manifest.
4. **Retain evidence:** record what was checked, what matched or failed, and the authority of the reference.
5. **Approve separately:** establish whether the reference itself was independently trusted.

An internally generated, self-consistent manifest is not proof that a build was authorized, that its inputs were safe, or that its builder was trustworthy.

## 2. Reference implementation and observed outcome

The reference implementation was created on branch `example/koppa-supply-chain-integrity` in `chadcoulter/Koppa_Libraries`.

- Workflow: `.github/workflows/example-integrity.yml`
- Integrity utility: `examples/supply_chain_integrity/koppa_integrity.py`
- Example usage: `examples/supply_chain_integrity/README.md`
- GitHub Actions run: https://github.com/chadcoulter/Koppa_Libraries/actions/runs/44843801164

That workflow successfully completed checkout, environment setup, package installation, existing regression tests, wheel and source-distribution generation, candidate-manifest creation, verification against that candidate manifest, and evidence upload.

**What the successful run establishes:** the example was able to build its software, record artifact identity, re-read the generated artifacts, validate their bytes against the just-produced candidate record, and retain the outputs. It did **not** establish independently approved source/dependency provenance, signed builder identity, or deployment trust.

The existing Koppa implementation uses SHA-256 and deferred function tokens. Other repositories can reproduce the sequence using an adapter with equivalent declared semantics; they do not have to use Python wheels.

## 3. Portable system boundaries

| Component | Responsibility | Required contract |
| --- | --- | --- |
| Source selector | Select the exact revision to build | Record an immutable source revision identifier |
| Build environment | Install tools and execute the build | Record relevant toolchain versions and configuration when available |
| Existing test runner | Exercise repository's established tests | A failed test blocks artifact acceptance in CI |
| Artifact enumerator | Locate the outputs of the completed build | Determine artifact type, filename, and byte length |
| Digest provider | Calculate bytes-to-digest identity | Specify digest algorithm, currently SHA-256 in reference example |
| Manifest writer | Serialize candidate artifact identity records | Deterministic field order/encoding and explicit candidate status |
| Verification adapter | Read files and compare observed identity against the manifest | Fail closed on missing files, invalid paths, malformed tokens, or mismatches |
| Evidence writer | Preserve per-artifact decisions | Record reference authority, observed digests, and failure reasons |
| CI artifact store | Retain build products and evidence | Store without interpreting candidate success as release approval |
| Approval authority (separate) | Accept an expected manifest digest or authenticated attestation | The trust anchor must be controlled independently of the evaluated build |

## 4. Reproducible implementation sequence

### Stage 1 — Isolate the work

Create an integration branch based on the project's intended development baseline. Add a nonpublishing CI workflow and a standalone verifier; leave production deployment and release publishing disabled. Keep the existing application/library logic unchanged when possible.

**Input:** repository and selected base revision.  
**Output:** reviewable branch containing the integration.

### Stage 2 — Check out a fixed source revision

CI obtains the exact commit that triggered the run. Preserve its identifier in build metadata or accompanying evidence. Use only the permissions necessary to read the source and write CI artifacts; the reference workflow sets `contents: read`.

**Input:** immutable source revision.  
**Output:** checked-out source tree.

### Stage 3 — Establish the build environment

Install the language runtime, dependencies, and packaging tools for the project. Use the repository's actual build instructions, not a universal command. Where supported, constrain toolchain and dependency versions so the job can be reproduced.

**Input:** project build configuration.  
**Output:** build-ready environment.

### Stage 4 — Execute existing tests

Run the project's normal regression suite *before* assembling release candidates. Treat any test failure as a pipeline failure. This is a build readiness gate, not a replacement for future language-independent Cucumber conformance tests.

**Input:** checked-out project and test runner.  
**Output:** pass/fail test result.

### Stage 5 — Build distributable artifacts

Produce the artifacts consumers would install or deploy: for example wheels/source distributions, Rust crates/binaries, JVM archives, Node packages, OCI images, or native executables. Each platform decides its artifact set. Record and retain the actual outputs; do not rely on filenames alone as identities.

**Input:** source, dependencies, toolchain.  
**Output:** one or more immutable-by-content build artifacts.

### Stage 6 — Generate a candidate integrity manifest

Enumerate each produced file and calculate identity from its **raw bytes**. In the reference example the candidate record contains:

- Schema identifier and explicit `UNAPPROVED_CANDIDATE` marker.
- Artifact filename and byte length.
- SHA-256 digest of the raw payload.
- Serialized Koppa function token resolving to that raw-payload digest.
- SHA-256 digest of the *handle-framed* representation, which is distinct from the raw-payload digest.

Serialize the manifest canonically: the reference uses sorted JSON keys, compact separators, ASCII-escaped text, and one trailing newline. Compute and report the SHA-256 of those exact manifest bytes.

**Input:** completed artifact files.  
**Output:** candidate manifest and its digest.

### Stage 7 — Verify by re-reading the artifacts

The verifier must accept an existing manifest, separately read every referenced artifact, calculate observed digests, and compare against expected values. The reference example also resolves its function token and verifies the associated framed representation.

At minimum, check file existence, safe filename/path resolution, byte length, declared digest format, duplicate manifest entries, actual digest equality, and consistent token/framing values when the integration supports them.

A mismatch, missing artifact, invalid manifest, or failed verification should return a nonzero process exit code so CI fails. Record per-artifact outcomes and reasons rather than returning a bare boolean.

**Input:** candidate or independently approved manifest plus actual artifacts.  
**Output:** verification result and detailed evidence.

### Stage 8 — Distinguish verification from approval

There are **two separate evidence meanings**:

- **Candidate integrity check:** a manifest and artifacts produced by the same build are mutually consistent. This is what the working initial CI example demonstrated. The reference labels it `CANDIDATE_INTEGRITY_CHECK_ONLY`.
- **Approved-reference check:** the manifest bytes are first matched against a digest supplied and protected by a separate approval authority, then the artifact files are checked against that manifest. The reference labels this `APPROVED_REFERENCE_CHECK` when an expected manifest digest was supplied and matched.

The second meaning is valid only when the supplied reference really was approved and retained outside the build's write-access boundary. A self-generated digest passed back into the same job does not create independent trust.

### Stage 9 — Retain machine-readable evidence

Store the build artifacts, candidate manifest, and verification evidence as CI artifacts. The reference upload uses a nonpublishing artifact action. Preserve exact source/run identifiers in associated CI metadata or enrich the schema in later integrations.

Do not publish a package, merge branches, deploy software, or claim release authorization solely because this stage succeeded.

**Output:** recoverable artifact set and evidence tied to the CI run.

## 5. Minimal abstract contracts

A reusable implementation can expose these conceptual operations:

```text
build(source_revision, environment) -> artifacts[]
record_candidate(artifacts[]) -> candidate_manifest
digest(candidate_manifest.bytes) -> manifest_digest
verify(artifacts[], manifest, optional_trusted_manifest_digest)
    -> {status, reference_type, per_artifact_results, evidence}
retain(artifacts[], candidate_manifest, evidence, run_metadata)
    -> storage_references
```

Implementations can be written in any language and can use any CI provider. The contract must specify which bytes are hashed and how a digest or token is serialized. For Koppa token interoperability, use the same handle, framing, hash algorithm, token encoding, and test vectors as the implementation being validated; identical method names alone do not guarantee byte-compatible results.

## 6. Evidence model

The demonstrated prototype uses a JSON evidence object with:

- `schema`: format/version identifier.
- `manifest_sha256`: observed digest of the manifest bytes.
- `reference`: `UNPINNED_MANIFEST` or `EXTERNALLY_PINNED_MANIFEST`.
- `status`: overall `PASS` or `FAIL`.
- `interpretation`: `CANDIDATE_INTEGRITY_CHECK_ONLY` or `APPROVED_REFERENCE_CHECK`.
- `artifacts`: each filename, result, failure reason, and observed digest when available.

A broader enterprise schema may add source commit, dependency lockfile digest, toolchain identities, attestation signatures, run ID, trusted approval reference, and deployment lineage. These extensions are **not implemented by the initial example** and must not be marked verified merely because a field exists.

## 7. Porting template for another repository

1. Identify the repository's source branch, test command, build command, and output directory.
2. Define the set of artifact types to enumerate; use a strict allowlist or explicit build outputs.
3. Implement or reuse a digest/token adapter and document its byte-level format.
4. Add a deterministic candidate-manifest generator that reads built outputs.
5. Add a *separate verification invocation* that reopens and checks the artifact bytes.
6. Use CI to run checkout → environment setup → tests → build → candidate record → verify → upload.
7. Confirm failure results make CI fail, including a tampered or missing artifact.
8. Keep release/deployment disabled until independently defined approval gates are satisfied.
9. Preserve sample manifests/evidence and the exact successful workflow run for reproducibility.
10. For an independent approval gate, place the trusted manifest digest or signed attestation outside the job's ability to rewrite it.

The first seven stages reproduce the demonstrated build-and-check mechanism. Steps 8–10 define operational boundaries and the path toward actual authenticated supply-chain approval.

## 8. Applicability across languages

| Ecosystem | Example build output | Adapter difference | Invariant process |
| --- | --- | --- | --- |
| Python | Wheel and source archive | Python packaging and file discovery | Build, hash raw bytes, manifest, re-verify, retain |
| Rust | Crate package or executable | Cargo build/package outputs | Same |
| Java/JVM | JAR or other JVM archive | Maven/Gradle artifact collection | Same |
| JavaScript/TypeScript | Package tarball or bundle | npm/pnpm and bundler outputs | Same |
| C/C++ | Executable, library, or archive | Compiler/build-system outputs | Same |
| Containerized apps | OCI image by digest | Registry/image-manifest inspection | Same, using defined image digest semantics |

The future Cucumber conformance layer should exercise the **same verification outcomes and evidence semantics** against all adapters. It is deliberately not part of the first working build sequence.

## 9. Acceptance requirements for the reusable example

A port reproduces the initial working process when:

- Its normal regression tests pass.
- At least one installable/deployable artifact is built.
- A deterministic candidate manifest records all selected output files.
- Re-verification reads the actual files and matches the declared raw-byte digests.
- Modified/missing artifacts lead to a failure result and nonzero exit status.
- CI retains both the built outputs and verification records.
- An unpinned manifest is labeled a **candidate**, never as independently approved.
- No release, merge, or deployment occurs as a side effect of successful verification.

The reference Koppa Actions run completed its normal execution, including artifact upload. The reference did not yet add automated tamper fixtures or cross-language conformance testing; those remain separate work items.

## 10. Reference links

- [Working integration workflow](https://github.com/chadcoulter/Koppa_Libraries/blob/example/koppa-supply-chain-integrity/.github/workflows/example-integrity.yml)
- [Working integrity utility](https://github.com/chadcoulter/Koppa_Libraries/blob/example/koppa-supply-chain-integrity/examples/supply_chain_integrity/koppa_integrity.py)
- [Example-specific operational README](https://github.com/chadcoulter/Koppa_Libraries/blob/example/koppa-supply-chain-integrity/examples/supply_chain_integrity/README.md)
- [Successful Actions execution](https://github.com/chadcoulter/Koppa_Libraries/actions/runs/44843801164)
- [Broader chain-of-trust design](https://github.com/chadcoulter/Koppa_Libraries/blob/main/docs/CI_CD_CHAIN_OF_TRUST.md)

---

**Reusable principle:** verify the actual shipped bytes, preserve the results, and keep *candidate consistency* distinct from *independent authorization*. The repository supplies the build implementation; the verification contract supplies reproducible meaning.
