export type EditableTargetDescription = {
  tagName: string
  inputType?: string
  isContentEditable?: boolean
  disabled?: boolean
}

export type EditableTargetKind = "input" | "textarea" | "contenteditable"

const textInputTypes = new Set([
  "email",
  "password",
  "search",
  "tel",
  "text",
  "url",
])

export function editableTargetKind(
  target: EditableTargetDescription
): EditableTargetKind | null {
  if (target.disabled) return null

  const tagName = target.tagName.toLowerCase()
  if (tagName === "textarea") return "textarea"
  if (tagName === "input") {
    return textInputTypes.has((target.inputType || "text").toLowerCase())
      ? "input"
      : null
  }
  return target.isContentEditable ? "contenteditable" : null
}

export function replaceTextSelection(
  value: string,
  selectionStart: number,
  selectionEnd: number,
  insertion: string
) {
  const start = Math.max(0, Math.min(selectionStart, value.length))
  const end = Math.max(start, Math.min(selectionEnd, value.length))
  return {
    value: `${value.slice(0, start)}${insertion}${value.slice(end)}`,
    caret: start + insertion.length,
  }
}

export function selectedTextFromRange(
  value: string,
  selectionStart: number,
  selectionEnd: number
) {
  const start = Math.max(0, Math.min(selectionStart, value.length))
  const end = Math.max(start, Math.min(selectionEnd, value.length))
  return value.slice(start, end)
}

export function editableMenuAvailability({
  selectedText,
  readOnly,
}: {
  selectedText: string
  readOnly: boolean
}) {
  return {
    canCopy: selectedText.length > 0,
    canPaste: !readOnly,
  }
}

export function clampMenuPosition({
  x,
  y,
  menuWidth,
  menuHeight,
  viewportWidth,
  viewportHeight,
  margin = 8,
}: {
  x: number
  y: number
  menuWidth: number
  menuHeight: number
  viewportWidth: number
  viewportHeight: number
  margin?: number
}) {
  const maxX = Math.max(margin, viewportWidth - menuWidth - margin)
  const maxY = Math.max(margin, viewportHeight - menuHeight - margin)
  return {
    x: Math.max(margin, Math.min(x, maxX)),
    y: Math.max(margin, Math.min(y, maxY)),
  }
}
