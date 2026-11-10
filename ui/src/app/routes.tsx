import { lazy, Suspense } from "react";
import { Navigate, type RouteObject } from "react-router";

import { Root } from "@/app/Root";
import { AppScreen } from "@/screens/apps/AppScreen";
import { AppsScreen } from "@/screens/apps/AppsScreen";
import { DotEditorScreen } from "@/screens/DotEditorScreen";
import { ConnectorsScreen } from "@/screens/connectors/ConnectorsScreen";
import { ChatRedirect } from "@/screens/home/ChatRedirect";
import { HomeScreen } from "@/screens/home/HomeScreen";
import { InboxScreen } from "@/screens/InboxScreen";
import { IntentsScreen } from "@/screens/IntentsScreen";
import { LibraryScreen } from "@/screens/library/LibraryScreen";
import { RunScreen } from "@/screens/RunScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { ReleasesScreen } from "@/screens/ReleasesScreen";
import { TeamScreen } from "@/screens/teams/TeamScreen";
import { TeamsScreen } from "@/screens/teams/TeamsScreen";
import { WorkflowBuilderScreen } from "@/screens/teams/builder/WorkflowBuilderScreen";
import { WorkflowRunScreen } from "@/screens/teams/run/WorkflowRunScreen";
import { UsageScreen } from "@/screens/UsageScreen";

// The Graph screen and its graph library load when it is first opened.
const GraphScreen = lazy(() => import("@/screens/graph/GraphScreen"));

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
      { path: "connectors", element: <ConnectorsScreen /> },
      { path: "inbox", element: <InboxScreen /> },
      { path: "intents", element: <IntentsScreen /> },
      { path: "apps", element: <AppsScreen /> },
      { path: "apps/:name", element: <AppScreen /> },
      { path: "chat", element: <Navigate to="/home" replace /> },
      { path: "chat/:dotId", element: <ChatRedirect /> },
      { path: "chat/:dotId/:runId", element: <ChatRedirect /> },
      { path: "teams", element: <TeamsScreen /> },
      { path: "teams/:teamId", element: <TeamScreen /> },
      { path: "teams/:teamId/workflows/:workflowId", element: <WorkflowBuilderScreen /> },
      { path: "workflow-runs/:runId", element: <WorkflowRunScreen /> },
      // Settings is one page per tab; the bare path opens the first tab.
      { path: "settings", element: <Navigate to="/settings/general" replace /> },
      { path: "settings/:tab", element: <SettingsScreen /> },
      { path: "usage", element: <UsageScreen /> },
      { path: "releases", element: <ReleasesScreen /> },
      {
        path: "graph/:dotId",
        element: (
          <Suspense fallback={<p className="p-6 text-sm text-muted-foreground">Loading…</p>}>
            <GraphScreen />
          </Suspense>
        ),
      },
      { path: "*", element: <Navigate to="/home" replace /> },
    ],
  },
];
