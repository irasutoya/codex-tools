import { Component, type ErrorInfo, type ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"

type ErrorBoundaryProps = {
  children: ReactNode
  label?: string
}

type ErrorBoundaryState = {
  error?: Error
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = {}

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("React render error:", error, info.componentStack)
  }

  private reset = () => {
    this.setState({ error: undefined })
  }

  render() {
    if (this.state.error) {
      return (
        <Alert variant="destructive" className="m-3">
          <AlertTitle>{this.props.label ?? "页面渲染出错"}</AlertTitle>
          <AlertDescription>
            <p className="mb-3 break-words">{this.state.error.message}</p>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={this.reset}
            >
              重试
            </Button>
          </AlertDescription>
        </Alert>
      )
    }
    return this.props.children
  }
}
