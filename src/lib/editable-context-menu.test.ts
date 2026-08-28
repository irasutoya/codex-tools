import { describe, expect, it } from "vitest"

import {
  clampMenuPosition,
  editableMenuAvailability,
  editableTargetKind,
  replaceTextSelection,
  selectedTextFromRange,
} from "./editable-context-menu"

describe("editable context menu helpers", () => {
  it("只识别可用的文本编辑目标", () => {
    expect(editableTargetKind({ tagName: "input", inputType: "text" })).toBe(
      "input"
    )
    expect(
      editableTargetKind({ tagName: "input", inputType: "password" })
    ).toBe("input")
    expect(editableTargetKind({ tagName: "textarea" })).toBe("textarea")
    expect(
      editableTargetKind({ tagName: "div", isContentEditable: true })
    ).toBe("contenteditable")
    expect(
      editableTargetKind({ tagName: "input", inputType: "checkbox" })
    ).toBeNull()
    expect(editableTargetKind({ tagName: "input", disabled: true })).toBeNull()
  })

  it("在光标位置粘贴", () => {
    expect(replaceTextSelection("abcd", 2, 2, "XY")).toEqual({
      value: "abXYcd",
      caret: 4,
    })
  })

  it("用粘贴内容替换选区", () => {
    expect(replaceTextSelection("abcd", 1, 3, "XY")).toEqual({
      value: "aXYd",
      caret: 3,
    })
  })

  it("复制当前选区，没有选区时禁用复制", () => {
    expect(selectedTextFromRange("abcd", 1, 3)).toBe("bc")
    expect(
      editableMenuAvailability({ selectedText: "bc", readOnly: false })
    ).toEqual({ canCopy: true, canPaste: true })
    expect(
      editableMenuAvailability({ selectedText: "", readOnly: false })
    ).toEqual({ canCopy: false, canPaste: true })
  })

  it("只读字段允许复制并禁止粘贴", () => {
    expect(
      editableMenuAvailability({ selectedText: "只读内容", readOnly: true })
    ).toEqual({ canCopy: true, canPaste: false })
  })

  it("把菜单限制在可视区域内", () => {
    expect(
      clampMenuPosition({
        x: 390,
        y: 290,
        menuWidth: 160,
        menuHeight: 88,
        viewportWidth: 400,
        viewportHeight: 300,
      })
    ).toEqual({ x: 232, y: 204 })
  })
})
