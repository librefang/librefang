import React from "react";

interface RootErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

export class RootErrorBoundary extends React.Component<
  { children: React.ReactNode },
  RootErrorBoundaryState
> {
  state: RootErrorBoundaryState = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): RootErrorBoundaryState {
    return { hasError: true, error };
  }

  render() {
    if (!this.state.hasError) return this.props.children;

    // This is the dashboard's last recovery surface. Keep it independent of
    // CSS and i18n because either subsystem may have caused the render failure.
    return (
      <div style={{ display: "flex", height: "100vh", alignItems: "center", justifyContent: "center" }}>
        <div style={{ textAlign: "center", padding: "2rem", maxWidth: "32rem" }}>
          <p style={{ fontSize: "1.125rem", fontWeight: 600, marginBottom: "0.5rem" }}>
            Something went wrong
          </p>
          <p style={{ fontSize: "0.875rem", color: "#6b7280", wordBreak: "break-word" }}>
            {this.state.error?.message ?? "An unexpected error occurred."}
          </p>
          <button
            onClick={() => window.location.reload()}
            style={{
              marginTop: "1rem",
              borderRadius: "0.75rem",
              backgroundColor: "#0ea5e9",
              padding: "0.625rem 1.5rem",
              fontSize: "0.875rem",
              fontWeight: 700,
              color: "#fff",
              border: "none",
              cursor: "pointer",
            }}
          >
            Reload
          </button>
        </div>
      </div>
    );
  }
}
