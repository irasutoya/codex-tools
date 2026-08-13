export type RequestGate = ReturnType<typeof createRequestGate>
export type RequestPriority = "background" | "read" | "scan"

type Request = {
  generation: number
  priority: number
  current: boolean
}

const priorities: Record<RequestPriority, number> = {
  background: 0,
  read: 1,
  scan: 2,
}

export function createRequestGate() {
  let generation = 0
  const active: Array<Request | undefined> = []
  const changeListeners = new Set<() => void>()

  const notifyChange = () => {
    const listeners = [...changeListeners]
    changeListeners.clear()
    for (const listener of listeners) listener()
  }

  return {
    begin(priority: RequestPriority = "read") {
      const level = priorities[priority]
      const request: Request = { generation, priority: level, current: true }

      if (active.some((candidate, index) => candidate && index > level)) {
        request.current = false
        return request
      }

      for (let index = 0; index <= level; index += 1) {
        const current = active[index]
        if (current) current.current = false
        active[index] = undefined
      }
      active[level] = request
      notifyChange()
      return request
    },
    finish(request: Request) {
      if (active[request.priority] === request) {
        active[request.priority] = undefined
        notifyChange()
      }
      request.current = false
    },
    invalidate() {
      generation += 1
      for (const request of active) {
        if (request) request.current = false
      }
      active.length = 0
      notifyChange()
    },
    isCurrent(request: Request) {
      return request.generation === generation && request.current
    },
    waitForChange() {
      return new Promise<void>((resolve) => {
        changeListeners.add(resolve)
      })
    },
  }
}
