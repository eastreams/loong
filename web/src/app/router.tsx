import { createBrowserRouter, Navigate, Outlet } from "react-router-dom";
import RootLayout from "./layouts/RootLayout";
import ChatPage from "../features/chat/pages/ChatPage";
import DashboardPage from "../features/dashboard/pages/DashboardPage";

function PageLayout() {
  return (
    <RootLayout>
      <Outlet />
    </RootLayout>
  );
}

export const router = createBrowserRouter([
  {
    path: "/",
    element: <PageLayout />,
    children: [
      {
        index: true,
        element: <Navigate replace to="/chat" />,
      },
      {
        path: "chat",
        element: <ChatPage />,
      },
      {
        path: "dashboard",
        element: <DashboardPage />,
      },
    ],
  },
]);
