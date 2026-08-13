import { describe, expect, it } from "vitest"

import { extractReleaseNotes } from "./extract-release-notes.mjs"

describe("extractReleaseNotes", () => {
  it("does not match a longer version with the same prefix", () => {
    const changelog = `
## [1.2.30] - 2026-01-02

- wrong release

## [1.2.3] - 2026-01-01

- expected release
`
    expect(extractReleaseNotes(changelog, "1.2.3")).toBe("- expected release\n")
  })

  it("stops at the next exact version section", () => {
    const changelog = `
## [1.2.3] - 2026-01-02

- expected release

## [1.2.2] - 2026-01-01

- older release
`
    expect(extractReleaseNotes(changelog, "1.2.3")).toBe("- expected release\n")
  })

  it("rejects an empty release section", () => {
    expect(() =>
      extractReleaseNotes(
        "## [1.2.3] - 2026-01-02\n\n## [1.2.2] - 2026-01-01\n",
        "1.2.3"
      )
    ).toThrow("没有发布说明")
  })
})
