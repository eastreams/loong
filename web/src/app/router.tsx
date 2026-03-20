import { createBrowserRouter, Navigate, useLocation } from "react-router-dom";
import RootLayout from "./layouts/RootLayout";
import ChatPage from "../features/chat/pages/ChatPage";
import DashboardPage from "../features/dashboard/pages/DashboardPage";

function WorkspaceLayout() {
  const location = useLocation();
  const activeSection = location.pathname.startsWith("/dashboard")
    ? "dashboard"
    : "chat";

  return (
    <RootLayout>
      <div hidden={activeSection !== "chat"} aria-hidden={activeSection !== "chat"}>
        <ChatPage />
      </div>
      <div
        hidden={activeSection !== "dashboard"}
        aria-hidden={activeSection !== "dashboard"}
      >
        <DashboardPage />
      </div>
    </RootLayout>
  );
}

export const router = createBrowserRouter([
  {
    path: "/",
    element: <WorkspaceLayout />,
    children: [
      {
        index: true,
        element: <Navigate replace to="/chat" />,
      },
      {
        path: "chat",
        element: null,
      },
      {
        path: "dashboard",
        element: null,
      },
    ],
  },
]);
