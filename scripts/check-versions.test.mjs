import { describe, expect, it } from "vitest"

import {
  checkVersions,
  isValidSemver,
  parseArgs,
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

describe("parseArgs", () => {
  it.each([
    [["--tag", "v1.2.3"], "v1.2.3"],
    [["--tag=v1.2.3-rc.1"], "v1.2.3-rc.1"],
  ])("reads a tag from %j", (args, tag) => {
    expect(parseArgs(args)).toEqual({ tag })
  })

  it("allows no arguments", () => {
    expect(parseArgs([])).toEqual({ tag: undefined })
  })

  it.each([
    [["--unknown"], "未知参数：--unknown"],
    [["release"], "不允许多余的位置参数：release"],
    [["--tag", "v1.2.3", "extra"], "不允许多余的位置参数：extra"],
    [["--tag"], "--tag 后必须提供标签"],
    [["--tag="], "--tag 后必须提供标签"],
    [["--tag", "--unknown"], "--tag 后必须提供标签"],
    [["--tag", "v1.2.3", "--tag=v1.2.3"], "--tag 不能重复提供"],
    [["--tag=v1.2.3", "--tag", "v1.2.3"], "--tag 不能重复提供"],
  ])("rejects invalid arguments %j", (args, message) => {
    expect(() => parseArgs(args)).toThrow(message)
  })
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

describe("version validation", () => {
  it("reports a missing Cargo.lock root version as a mismatch", () => {
    expect(() =>
      checkVersions({
        expected: "1.2.3",
        versions: { "Cargo.lock (root package)": undefined },
      })
    ).toThrow("Cargo.lock (root package): <missing>")
  })
})
