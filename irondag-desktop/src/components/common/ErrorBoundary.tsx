import { Component, ErrorInfo, ReactNode } from "react";

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

const isProduction = import.meta.env.PROD;

export class ErrorBoundary extends Component<Props, State> {
  public state: State = {
    hasError: false,
    error: null
  };

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("Uncaught error:", error, errorInfo);
  }

  private handleRetry = () => {
    this.setState({ hasError: false, error: null });
  };

  public render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }

      return (
        <div
          style={{
            padding: "2rem",
            background: "#1e1e2e",
            border: "1px solid rgba(239, 68, 68, 0.3)",
            borderRadius: 16,
            textAlign: "center",
            boxShadow: "0 8px 32px rgba(0, 0, 0, 0.4)",
            backdropFilter: "blur(12px)"
          }}
        >
          <div style={{
            fontSize: "3rem",
            marginBottom: "1rem"
          }}>
            ⚠️
          </div>
          <h3 style={{
            color: "#fca5a5",
            marginBottom: "1rem",
            fontSize: "1.5rem",
            fontWeight: "600"
          }}>
            Something went wrong
          </h3>
          <p style={{
            color: "#94a3b8",
            marginBottom: "1.5rem",
            fontSize: "0.95rem",
            lineHeight: "1.5"
          }}>
            {this.state.error?.message || "An unexpected error occurred"}
          </p>
          {!isProduction && this.state.error?.stack && (
            <details style={{
              marginBottom: "1.5rem",
              textAlign: "left",
              background: "rgba(0, 0, 0, 0.3)",
              padding: "1rem",
              borderRadius: 8,
              fontSize: "0.75rem",
              color: "#64748b",
              maxHeight: "150px",
              overflow: "auto"
            }}>
              <summary style={{ cursor: "pointer", color: "#94a3b8" }}>
                Stack trace (dev only)
              </summary>
              <pre style={{
                marginTop: "0.5rem",
                whiteSpace: "pre-wrap",
                wordBreak: "break-all"
              }}>
                {this.state.error.stack}
              </pre>
            </details>
          )}
          <button
            onClick={this.handleRetry}
            style={{
              padding: "0.75rem 2rem",
              borderRadius: 10,
              border: "none",
              background: "linear-gradient(135deg, #6366f1, #4f46e5)",
              color: "white",
              cursor: "pointer",
              fontWeight: "600",
              fontSize: "1rem",
              boxShadow: "0 4px 12px rgba(99, 102, 241, 0.4)",
              transition: "transform 0.2s ease, box-shadow 0.2s ease"
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.transform = "translateY(-2px)";
              e.currentTarget.style.boxShadow = "0 6px 16px rgba(99, 102, 241, 0.5)";
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.transform = "translateY(0)";
              e.currentTarget.style.boxShadow = "0 4px 12px rgba(99, 102, 241, 0.4)";
            }}
          >
            Try Again
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
