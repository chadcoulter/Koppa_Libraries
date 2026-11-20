# KOPPA — Building a Verifiable Software Supply Chain

## CI/CD Chain of Trust, Package Provenance, Deployment Tracking, and Operational Intelligence

**REDΣ Ecosystems Research Collective**  
**Technical Implementation Concept — November 2026**

---

## 1. Responsibility Begins With the Organization Building the Software

**Koppa provides a computational mechanism for verification. It does not make deployment decisions, certify that code is safe, or assume responsibility for the systems in which it is implemented.**

The purpose of this release is to give organizations the tools necessary to establish whether the software they are building, importing, modifying, and deploying is the software they intended to use.

Koppa does not replace engineering judgment.

It does not determine whether a package should be trusted merely because its hash is valid.

It does not decide which code belongs in production.

It does not relieve developers, security engineers, vendors, or enterprise operators of responsibility for understanding the systems they operate.

**The objective is not to provide another button that says Deploy. The objective is to provide the information necessary to understand what that button is deploying.**

Every organization adopting Koppa remains responsible for defining its own trust policies, selecting acceptable sources, verifying its build process, investigating anomalies, and determining appropriate remediation.

Koppa provides the mechanisms through which those decisions can become traceable, reproducible, and independently verifiable.

---

## 2. Establishing the Chain of Trust

Modern software applications are assembled from numerous packages, frameworks, libraries, containers, build tools, and external dependencies.

A single deployment may incorporate hundreds or thousands of components that were not written by the organization deploying them.

Each component introduces a relationship between a source, a package, a build process, and an executable artifact.

Koppa provides a means of preserving those relationships throughout the software lifecycle.

### Conceptual verification chain

\`\`\`text
APPROVED SOURCE REPOSITORY
           |
           v
DEPENDENCY DECLARATION
           |
           v
PACKAGE SOURCE VERIFICATION
           |
           v
PACKAGE CONTENT VALIDATION
           |
           v
BUILD ENVIRONMENT VERIFICATION
           |
           v
COMPILED ARTIFACT VERIFICATION
           |
           v
RELEASE / DEPLOYMENT VERIFICATION
           |
           v
RUNTIME VERSION IDENTIFICATION
           |
           v
LOGS + METRICS + TRACES
           |
           v
HISTORICAL ANALYSIS
\`\`\`

Each stage records what was expected, what was observed, and the relationship between them.

A successful verification establishes continuity with an approved reference. A failed verification identifies a point requiring investigation.

The entire chain remains available for later inspection.

---

## 3. Verifying Every Imported Package

A package name and version number are not sufficient to establish package identity.

Consider a hypothetical application importing:

\`\`\`text
application
  ├── authentication-library  2.4.1
  ├── cryptographic-provider   3.2.0
  ├── database-client          1.8.7
  └── telemetry-agent          4.1.0
\`\`\`

Each package may introduce additional transitive dependencies.

A complete inventory therefore includes both direct and indirect dependencies.

### Package identity record

For each package, a CI/CD implementation could retain:

| Field | Purpose |
|---|---|
| Package name | Logical package identifier |
| Package version | Declared release |
| Registry / source | Where the package originated |
| Source repository | Associated source-code location |
| Source commit | Exact source revision, when available |
| Package SHA-256 | Exact imported package bytes |
| Approved SHA-256 | Expected package identity |
| Build provenance | Origin and build-process evidence |
| Parent dependencies | Which packages introduced it |
| Validation outcome | Accepted, rejected, or unresolved |
| Observation time | When verification occurred |

A package manifest containing these records establishes a relationship between the software's declared dependency graph and the actual bytes entering the build.

### Example

\`\`\`text
Package: authentication-library
Version: 2.4.1

Registry: approved-registry
Expected SHA-256: A7F2...
Observed SHA-256: A7F2...

Source identity: VERIFIED
Package identity: VERIFIED
Provenance: ACCEPTED
\`\`\`

If the same name and version subsequently produce different package contents:

\`\`\`text
Package: authentication-library
Version: 2.4.1

Expected SHA-256: A7F2...
Observed SHA-256: B91C...

Package identity: MISMATCH
\`\`\`

The verification now provides a concrete reason for investigation.

The package may have been republished, substituted, corrupted, or altered through a compromised distribution channel.

The discrepancy is evidence of changed identity; it is not, by itself, proof of malicious activity.

**The important point is that a package cannot silently become a different package while retaining an accepted identity.**

---

## 4. Applying the Koppa Swap

Koppa introduces deferred relational representation as a mechanism for retaining information until it is needed for resolution.

In Koppa 0.1.1, the \`FunctionToken\` implementation demonstrates this principle through deferred digest representation.

Instead of embedding a digest into the very data used to calculate that digest, Koppa substitutes a constant handle and retains the digest separately.

The current implementation includes:

- \`FunctionToken\`
- \`tokenized_payload()\`
- \`seal()\`
- \`verify()\`
- \`fixed_point_digest()\`

This separates the representation being hashed from the digest used to verify the underlying payload.

### Application to build integrity

A future build-verification integration could maintain three related records:

**1. Source identity**

The approved source revision and dependency inventory.

**2. Build identity**

The build environment, configuration, tools, and inputs associated with a particular execution.

**3. Artifact identity**

The digest of the resulting binary, package, or container image.

Conceptually:

\`\`\`text
SOURCE IDENTITY
      |
      v
BUILD INPUT RECORD
      |
      v
BUILD EXECUTION
      |
      v
ARTIFACT DIGEST
      |
      v
KOPPA VERIFICATION RECORD
\`\`\`

Koppa provides a way to preserve and resolve selected identity relationships.

A CI/CD integration would additionally need an independently trusted approval record, authenticated build provenance, and verification policies.

These are implementation responsibilities rather than automatic properties of \`FunctionToken\`.

---

## 5. Establishing Verification Gates in CI/CD

An engineering organization can incorporate a sequence of verification gates into its existing pipeline.

| Stage | Verification question | Evidence retained |
|---|---|---|
| Source checkout | Is this the approved commit? | Repository and commit ID |
| Dependency resolution | Are these the approved packages? | Dependency graph and digests |
| Build preparation | Are the expected build inputs present? | Toolchain and configuration |
| Build execution | Which inputs produced this artifact? | Build provenance |
| Artifact publication | Is the artifact unchanged? | Artifact digest and attestation |
| Deployment | Is this the approved artifact? | Deployment and artifact IDs |
| Runtime | Which artifact is actually running? | Runtime version and image identity |

Each gate produces a verification outcome and records the identity relationships involved.

The organization's policy determines whether a failure blocks deployment, initiates investigation, or requires an authorized exception.

The important distinction is between **verification evidence** and **permission to deploy**.

Those are separate decisions.

### Compatibility with established standards

Koppa-based verification can coexist with Software Bills of Materials (SBOM), SLSA build provenance, in-toto attestations, signed artifacts, and existing enterprise CI/CD systems.

These mechanisms can establish source, builder, artifact, and signing identity without requiring organizations to replace their existing delivery platforms.

---

## 6. Building a Historical Package Lineage

Once each build records its complete package inventory, those records can be connected to deployment history.

This creates a searchable relationship between dependencies and every environment in which they appeared.

For example:

\`\`\`text
authentication-library 2.4.1
          |
          | SHA-256: A7F2...
          |
          +--- Build 1042
          |       |
          |       +--- Development
          |       +--- QA
          |
          +--- Build 1047
          |       |
          |       +--- Production
          |
          +--- Build 1051
                  |
                  +--- Production / Region B
\`\`\`

If a package digest is subsequently identified as compromised, the organization can search the recorded dependency inventories and deployment histories.

The result is an impact inventory identifying:

- Which builds included the affected package.
- Which applications incorporated those builds.
- Which environments received those applications.
- When affected versions entered and left service.
- Which systems may still be running affected artifacts.
- Which later releases removed or replaced the package.

This changes the investigation from searching for a suspicious package name to querying an exact software identity.

### A compromised package investigation

Suppose the organization discovers that the package with digest \`A7F2...\` is compromised.

The question becomes:

**Where did these exact package bytes enter our software, and where did the resulting artifacts go?**

The dependency graph identifies the affected builds.

The deployment records identify the environments.

Runtime inventory identifies the potentially affected systems.

Operational telemetry helps determine what occurred while those systems were running.

The answer is assembled from retained relationships rather than reconstructed from memory.

---

## 7. Correlating Package Identity With Dynatrace

Dynatrace provides several existing capabilities relevant to this model.

Its release-monitoring functionality associates deployed software versions with deployment activity, operational problems, and vulnerabilities.

Its Runtime Vulnerability Analytics can identify vulnerable third-party components in monitored applications.

Koppa's role would be to contribute the exact package and build identity records needed for correlation.

### Example integration

\`\`\`text
KOPPA VALIDATION RECORD
       |
       +--- Build ID
       +--- Artifact Digest
       +--- Package Inventory
       +--- Verification Outcome
       |
       v
CI/CD DEPLOYMENT EVENT
       |
       v
DYNATRACE RELEASE MONITORING
       |
       +--- Deployed Version
       +--- Service / Process
       +--- Environment
       +--- Problems
       +--- Vulnerabilities
       |
       v
CORRELATED INVESTIGATION
\`\`\`

A pipeline can send deployment events into Dynatrace as SDLC events.

The organization can retain Koppa verification identifiers as additional metadata or in an externally queryable evidence record.

Dynatrace's version detection and deployment events are distinct inputs: a deployment event records the pipeline action, while version metadata identifies the software running in the environment.

### Practical investigation

If a package becomes suspect, the team could:

1. Identify every build containing the affected package digest.
2. Associate each build with its deployment identifiers.
3. Locate the corresponding releases and services in Dynatrace.
4. Examine operational problems associated with those releases.
5. Correlate vulnerability findings with affected services.
6. Determine which environments and deployments warrant investigation.

The result is an evidence-supported relationship between an identified software component and observed production conditions.

---

## 8. Correlating Package Identity With Datadog

Datadog's deployment tracking, CI visibility, application performance monitoring, logs, and tracing provide another route for correlating software identity with runtime behavior.

Datadog's unified service tagging supports the use of a \`version\` identifier across services, infrastructure metrics, traces, and logs.

A Koppa integration could associate that version with the underlying validated artifact digest and dependency inventory.

### Example relationship

\`\`\`text
KOPPA PACKAGE INVENTORY
       |
       v
BUILD ID + ARTIFACT SHA-256
       |
       v
DATADOG SERVICE VERSION
       |
       +--- Deployment Time
       +--- Application Logs
       +--- Error Rates
       +--- Distributed Traces
       +--- Infrastructure Metrics
       |
       v
HISTORICAL COMPARISON
\`\`\`

The organization can then compare service behavior before and after the deployment of an artifact containing a particular package version.

### Example investigation

A new version of a database client enters production.

Shortly afterward, monitoring records an increase in connection failures.

The package inventory identifies which builds included the new client.

Deployment records identify when those builds entered each environment.

Datadog provides the runtime error and performance information associated with those deployments.

The engineering team can investigate whether the observed changes correspond to the new dependency.

**Correlation supplies evidence for investigation. It does not substitute for establishing causation.**

Datadog's Continuous Delivery Visibility can provide additional deployment history where the organization has access to that feature.

---

## 9. Identifying Trends Across Deployments

The value of preserving package identity extends beyond responding to an individual compromised dependency.

Over time, retained verification records can reveal patterns involving package sources, release processes, and observed operational failures.

An enterprise could track:

| Trend | What it reveals |
|---|---|
| Digest changes under unchanged version labels | Unexpected package-content changes |
| Repeated provenance failures | Sources or build processes requiring investigation |
| Vulnerabilities grouped by registry | Concentrations associated with particular sources |
| Error increases after dependency changes | Operational changes associated with package introductions |
| Repeated deployment of affected packages | Failures in dependency controls or remediation |
| Build-to-runtime identity mismatches | Discrepancies between expected and running software |
| Increasing unresolved verification events | Deterioration in evidence completeness |

### Source-level patterns

Consider an organization using packages from multiple registries.

Its retained records might show that one source repeatedly produces verification discrepancies.

That observation gives security engineers a specific source to investigate.

Alternatively, multiple applications may experience similar failures after importing the same package version.

A shared dependency graph can expose that common relationship even when the applications are owned by different teams.

### Temporal patterns

The records can also establish when changes occurred.

For example:

\`\`\`text
November 2  — Package introduced
November 3  — First production deployment
November 4  — Error increase observed
November 5  — Additional services affected
November 6  — Package flagged for investigation
November 7  — Replacement deployed
November 8  — Error rates return to baseline
\`\`\`

This combines package provenance with operational history.

Such records provide a foundation for incident reconstruction, trend analysis, and engineering accountability.

---

## 10. The Long-Term Value: A Relational History of Software

A package is not simply a filename, a version string, or a hash.

It participates in a continuing relationship with the systems that import, build, distribute, and execute it.

Koppa's broader mathematical model emphasizes preservation of those relationships through transformation.

Applied to software delivery, this suggests retaining connections among:

**Source → Dependency → Build → Artifact → Deployment → Runtime → Observation**

The resulting history can support both forward and reverse investigation.

**Forward tracing:** Given a package, identify every downstream artifact, deployment, and affected service.

**Reverse tracing:** Given a runtime anomaly, identify the deployed artifact, its build, and the dependencies that participated in producing it.

An organization can then evaluate suspicious changes in the context of the software's actual history.

This is more useful than relying on isolated checks that provide no continuing connection to the systems they were intended to protect.

---

## 11. Implementation Scope

Koppa 0.1.1 currently provides deferred relational operations, exact comparisons, associative scan functions, and function-token digest representation.

The complete CI/CD chain described in this document is a proposed application architecture.

It would require integration with existing systems responsible for:

- Recording dependency inventories and build provenance.
- Authenticating approval and attestation records.
- Associating compiled artifacts with package inventories.
- Retaining deployment identities and histories.
- Exporting validation results to observability systems.
- Querying historical relationships across software releases.

Koppa need not replace existing tools performing those functions.

It can provide computational mechanisms that support the preservation and verification of their relationships.

The implementation and operation of those systems remain the responsibility of the adopting organization.

---

## 12. Closing Principle

The purpose of Koppa is not to decide whether software is trustworthy.

Its purpose is to give developers and engineers the ability to determine whether the software in front of them is the software they intended to build, whether its constituent packages retain their expected identities, and whether its history can be reconstructed after deployment.

An organization should understand what it is introducing into its environment, what evidence supports that decision, and what consequences follow.

**Koppa provides the verification capability.**

**The organization retains the responsibility.**

---

**REDΣ Ecosystems Research Collective**  
Koppa Libraries — Defensive Computational Research

Copyright © 2026 REDΣ Ecosystems Research Collective.

Licensed under the applicable Koppa Software License and Patent Non-Appropriation Addendum.
