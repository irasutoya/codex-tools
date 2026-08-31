// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { PaginationNext, PaginationPrevious } from "@/components/ui/pagination"

describe("pagination controls", () => {
  it("uses native disabled buttons at page boundaries", () => {
    render(
      <>
        <PaginationPrevious disabled />
        <PaginationNext disabled />
      </>
    )

    expect(
      (screen.getByRole("button", { name: "转到上一页" }) as HTMLButtonElement)
        .disabled
    ).toBe(true)
    expect(
      (screen.getByRole("button", { name: "转到下一页" }) as HTMLButtonElement)
        .disabled
    ).toBe(true)
  })
})
