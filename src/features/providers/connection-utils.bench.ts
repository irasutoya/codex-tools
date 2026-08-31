import { bench, describe } from "vitest"

import {
  effectiveModelCount,
  effectiveModelsOf,
  emptyProvider,
} from "@/features/providers/connection-utils"

const provider = {
  ...emptyProvider(),
  availableModels: Array.from(
    { length: 10_000 },
    (_, index) => `model-${index}`
  ),
  customModels: Array.from(
    { length: 5_000 },
    (_, index) => `model-${index + 7_500}`
  ),
}

describe("provider model catalog", () => {
  bench("builds a deduplicated effective model list", () => {
    effectiveModelsOf(provider)
  })

  bench("counts deduplicated effective models", () => {
    effectiveModelCount(provider)
  })
})
