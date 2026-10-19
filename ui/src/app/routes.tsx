import { Navigate, type RouteObject } from "react-router";

import { Root } from "@/app/Root";
import { ChatScreen } from "@/screens/ChatScreen";
import { DotEditorScreen } from "@/screens/DotEditorScreen";
import { DotsScreen } from "@/screens/DotsScreen";
import { InboxScreen } from "@/screens/InboxScreen";
import { LibraryScreen } from "@/screens/library/LibraryScreen";
import { RunScreen } from "@/screens/RunScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Root />,
    children: [
      { index: true, element: <Navigate to="/dots" replace /> },
      { path: "dots", element: <DotsScreen /> },
      { path: "dots/new", element: <DotEditorScreen /> },
      { path: "dots/:dotId", element: <DotEditorScreen /> },
      { path: "runs/:runId", element: <RunScreen /> },
      { path: "library", element: <LibraryScreen /> },
      { path: "inbox", element: <InboxScreen /> },
      { path: "chat", element: <ChatScreen /> },
      { path: "chat/:dotId", element: <ChatScreen /> },
      { path: "chat/:dotId/:runId", element: <ChatScreen /> },
      { path: "settings", element: <SettingsScreen /> },
      { path: "*", element: <Navigate to="/dots" replace /> },
    ],
  },
];
