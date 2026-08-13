import { describe, expect, it } from "vitest"

import {
  checkVersions,
  findVersionHeading,
  hasUnreleasedContent,
  isValidSemver,
  readCargoLockPackageVersion,
  readCargoPackageVersion,
} from "./check-versions.mjs"

describe("isValidSemver", () => {
  it.each([
    "0.0.0",
    "1.2.3",
    "1.2.3-rc.1",
    "1.2.3-0A",
    "1.2.3+build.7",
    "1.2.3-rc.1+build.7",
  ])("accepts SemVer 2.0 version %s", (version) => {
    expect(isValidSemver(version)).toBe(true)
  })

  it.each(["01.2.3", "1.02.3", "1.2.03", "1.2.3-01", "1.2", "v1.2.3"])(
    "rejects invalid version %s",
    (version) => {
      expect(isValidSemver(version)).toBe(false)
    }
  )
})

describe("version sources", () => {
  it("reads only the Cargo.toml package section", () => {
    expect(
      readCargoPackageVersion(
        '[package]\nname = "app"\nversion = "1.2.3"\n\n[dependencies]\nversion = "9"\n'
      )
    ).toBe("1.2.3")
  })

  it("reads a Cargo.toml package section at end of file", () => {
    expect(
      readCargoPackageVersion('[package]\nname = "app"\nversion = "1.2.3"\n')
    ).toBe("1.2.3")
  })

  it("reads the source-free root package from Cargo.lock", () => {
    const lockfile = `
[[package]]
name = "app"
version = "1.2.3"

[[package]]
name = "dependency"
version = "4.5.6"
source = "registry+https://example.com"
`
    expect(readCargoLockPackageVersion(lockfile, "app")).toBe("1.2.3")
    expect(readCargoLockPackageVersion(lockfile, "missing")).toBeUndefined()
  })
})

describe("CHANGELOG validation", () => {
  it("matches an exact version heading instead of a version prefix", () => {
    expect(
      findVersionHeading(["## [1.2.30] - 2026-01-01"], "1.2.3")
    ).toBeUndefined()
    expect(
      findVersionHeading(["## [1.2.3] - 2026-01-01"], "1.2.3")
    ).toBeDefined()
  })

  it.each(["### Added", "release prose", "<!-- pending -->", "- release item"])(
    "detects non-empty Unreleased content: %s",
    (content) => {
      expect(
        hasUnreleasedContent(
          `## [Unreleased]\n\n${content}\n\n## [1.2.3] - 2026-01-01\n`
        )
      ).toBe(true)
    }
  )

  it("accepts an empty Unreleased section", () => {
    expect(
      hasUnreleasedContent("## [Unreleased]\n\n## [1.2.3] - 2026-01-01\n")
    ).toBe(false)
  })

  it("does not treat link references as Unreleased content", () => {
    expect(
      hasUnreleasedContent(
        "## [Unreleased]\n\n[Unreleased]: https://example.com/compare\n"
      )
    ).toBe(false)
  })

  it("reports a missing Cargo.lock root version as a mismatch", () => {
    expect(() =>
      checkVersions({
        expected: "1.2.3",
        versions: { "Cargo.lock (root package)": undefined },
      })
    ).toThrow("Cargo.lock (root package): <missing>")
  })
})
