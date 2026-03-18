import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import { OverviewPage } from "./pages/OverviewPage";

export function App() {
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-dot" />
          <div>
            <strong>LibreFang</strong>
            <p>React Dashboard</p>
          </div>
        </div>
        <nav>
          <NavLink to="/overview">Overview</NavLink>
        </nav>
      </aside>
      <main className="content">
        <Routes>
          <Route path="/" element={<Navigate to="/overview" replace />} />
          <Route path="/overview" element={<OverviewPage />} />
        </Routes>
      </main>
    </div>
  );
}
