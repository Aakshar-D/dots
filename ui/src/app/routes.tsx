import { Navigate, type RouteObject } from "react-router";

import { Root } from "@/app/Root";
import { DotEditorScreen } from "@/screens/DotEditorScreen";
import { ChatRedirect } from "@/screens/home/ChatRedirect";
import { HomeScreen } from "@/screens/home/HomeScreen";
import { InboxScreen } from "@/screens/InboxScreen";
import { LibraryScreen } from "@/screens/library/LibraryScreen";
import { RunScreen } from "@/screens/RunScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { UsageScreen } from "@/screens/UsageScreen";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Root />,
    children: [
      { index: true, element: <Navigate to="/home" replace /> },
      { path: "home", element: <HomeScreen /> },
      { path: "home/:dotId", element: <HomeScreen /> },
      { path: "home/:dotId/:rootRunId", element: <HomeScreen /> },
      // The Dots list and the Chat screen were folded into the home.
      { path: "dots", element: <Navigate to="/home" replace /> },
      { path: "dots/new", element: <DotEditorScreen /> },
      { path: "dots/:dotId", element: <DotEditorScreen /> },
      { path: "runs/:runId", element: <RunScreen /> },
      { path: "library", element: <LibraryScreen /> },
      { path: "inbox", element: <InboxScreen /> },
      { path: "chat", element: <Navigate to="/home" replace /> },
      { path: "chat/:dotId", element: <ChatRedirect /> },
      { path: "chat/:dotId/:runId", element: <ChatRedirect /> },
      { path: "settings", element: <SettingsScreen /> },
      { path: "usage", element: <UsageScreen /> },
      { path: "*", element: <Navigate to="/home" replace /> },
    ],
  },
];
