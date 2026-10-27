import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import {
  House,
  Inbox as InboxIcon,
  LayoutGrid,
  Library as LibraryIcon,
  Settings as SettingsIcon,
} from "lucide-react";
import { NavLink, Outlet } from "react-router";

import { commands } from "@/api/commands";
import { Banners } from "@/app/Banners";
import { LimitsMeter } from "@/components/LimitsMeter";
import { Badge } from "@/components/ui/badge";
import { Toaster } from "@/components/ui/sonner";
import { isOpenQuestion, waits } from "@/lib/messages";
import { keys } from "@/lib/query";
import { useRuntimeEvents } from "@/lib/runtime-events";

const LINKS = [
  { to: "/home", label: "Home", icon: House },
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/apps", label: "Apps", icon: LayoutGrid },
  { to: "/library", label: "Library", icon: LibraryIcon },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
];

/// The frame around every screen: the icon rail, the banners, the screen, the toasts.
export function Layout() {
  useRuntimeEvents();
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  }).data;
  // What the dots have for the person, read or not (needs-you spec section 5).
  const unread = useQuery({ queryKey: keys.unreadTotal, queryFn: commands.unreadTotal }).data ?? 0;
  // The same count as the tray's (`inbox_count`), the open questions and unread notes included.
  const waiting =
    (inbox
      ? inbox.approvals.length +
        inbox.review_runs.length +
        inbox.attention.length +
        inbox.tools_changed.length +
        inbox.watchers.length +
        // Runs held for Claude usage wait as one line, however many.
        Math.min(inbox.held.length, 1) +
        // A waiting app review counts like an approval.
        inbox.app_reviews.length
      : 0) + (messages ?? []).filter(waits).length;
  // A pending approval or an open question needs the person: the counts take the needs colour
  // (deep-teal spec section 4).
  const needs =
    (inbox?.approvals.length ?? 0) > 0 ||
    (inbox?.app_reviews.length ?? 0) > 0 ||
    (messages ?? []).some(isOpenQuestion);
  return (
    <div className="flex h-screen bg-background text-sm text-foreground">
      <nav
        aria-label="Main"
        className="flex w-[46px] shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-2 text-sidebar-foreground"
      >
        {LINKS.map(({ to, label, icon: Icon }) => {
          const count = to === "/inbox" ? waiting : to === "/home" ? unread : 0;
          // The name says the count too, and the tooltip shows it.
          const name = count > 0 ? `${label}, ${count} ${to === "/home" ? "unread" : "waiting"}` : label;
          return (
            <NavLink
              key={to}
              to={to}
              aria-label={name}
              title={name}
              className={({ isActive }) =>
                cn(
                  "relative flex size-9 items-center justify-center rounded-lg text-muted-foreground hover:bg-sidebar-accent hover:text-foreground",
                  isActive && "bg-sidebar-accent text-primary hover:text-primary",
                )
              }
            >
              <Icon className="size-5" aria-hidden />
              {count > 0 && (
                <Badge
                  data-tone={needs ? "needs" : "muted"}
                  className={cn(
                    "absolute -top-1 -right-1 h-4 min-w-4 px-1 text-[10px]",
                    needs ? "bg-needs text-needs-foreground" : "bg-muted text-muted-foreground",
                  )}
                >
                  {count > 99 ? "99+" : count}
                </Badge>
              )}
            </NavLink>
          );
        })}
        <LimitsMeter />
      </nav>
      <div className="flex min-w-0 flex-1 flex-col">
        <Banners />
        <main className="min-h-0 flex-1 overflow-auto">
          <Outlet />
        </main>
      </div>
      <Toaster />
    </div>
  );
}
