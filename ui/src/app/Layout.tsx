import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Bot, Inbox as InboxIcon, Library as LibraryIcon, MessageSquare, Settings as SettingsIcon } from "lucide-react";
import { NavLink, Outlet } from "react-router";

import { commands } from "@/api/commands";
import { Banners } from "@/app/Banners";
import { Badge } from "@/components/ui/badge";
import { Toaster } from "@/components/ui/sonner";
import { waits } from "@/lib/messages";
import { keys } from "@/lib/query";
import { useRuntimeEvents } from "@/lib/runtime-events";

const LINKS = [
  { to: "/dots", label: "Dots", icon: Bot },
  { to: "/library", label: "Library", icon: LibraryIcon },
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/chat", label: "Chat", icon: MessageSquare },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
];

/// The frame around every screen: the sidebar, the banners, the screen, the toasts.
export function Layout() {
  useRuntimeEvents();
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  }).data;
  // The same count as the tray's (`inbox_count`), the open questions and unread notes included.
  const waiting =
    (inbox
      ? inbox.approvals.length +
        inbox.review_runs.length +
        inbox.attention.length +
        inbox.tools_changed.length
      : 0) + (messages ?? []).filter(waits).length;
  return (
    <div className="flex h-screen bg-background text-sm text-foreground">
      <nav
        aria-label="Main"
        className="flex w-44 shrink-0 flex-col gap-1 border-r bg-sidebar p-2 text-sidebar-foreground"
      >
        <div className="px-2 py-3 text-base font-semibold">Dots</div>
        {LINKS.map(({ to, label, icon: Icon }) => (
          <NavLink
            key={to}
            to={to}
            className={({ isActive }) =>
              cn(
                "flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-sidebar-accent",
                isActive && "bg-sidebar-accent font-medium",
              )
            }
          >
            <Icon className="size-4" aria-hidden />
            <span className="flex-1">{label}</span>
            {to === "/inbox" && waiting > 0 && <Badge>{waiting}</Badge>}
          </NavLink>
        ))}
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
