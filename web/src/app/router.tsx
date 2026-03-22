import { lazy } from "react";
import { createBrowserRouter, Navigate } from "react-router-dom";
import RootLayout from "./layouts/RootLayout";

const ChatPage = lazy(() => import("../features/chat/pages/ChatPage"));
const DashboardPage = lazy(() => import("../features/dashboard/pages/DashboardPage"));

export const router = createBrowserRouter([
  {
    path: "/",
    element: <RootLayout />,
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
