import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import {
  Gauge,
  House,
  Inbox as InboxIcon,
  LayoutGrid,
  Library as LibraryIcon,
  Plug,
  Rocket,
  Search,
  Settings as SettingsIcon,
  Target,
  UsersRound,
} from "lucide-react";
import { useEffect, useState } from "react";
import { NavLink, Outlet } from "react-router";

import { commands } from "@/api/commands";
import { AccountSwitcher } from "@/app/AccountSwitcher";
import { Banners } from "@/app/Banners";
import { isSearchShortcut, SearchDialog } from "@/app/SearchDialog";
import { LimitsMeter } from "@/components/LimitsMeter";
import { Badge } from "@/components/ui/badge";
import { Toaster } from "@/components/ui/sonner";
import { inboxWaiting, shownMessages } from "@/lib/inbox";
import { isOpenQuestion } from "@/lib/messages";
import { keys } from "@/lib/query";
import { useRuntimeEvents } from "@/lib/runtime-events";
import { useActivity } from "@/lib/use-activity";
import { useVoice } from "@/lib/use-voice";

const LINKS = [
  { to: "/home", label: "Home", icon: House },
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/intents", label: "Intents", icon: Target },
  { to: "/apps", label: "Apps", icon: LayoutGrid },
  { to: "/library", label: "Library", icon: LibraryIcon },
  { to: "/teams", label: "Teams", icon: UsersRound },
  { to: "/connectors", label: "Connectors", icon: Plug },
  { to: "/usage", label: "Usage", icon: Gauge },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
];
/// Only a publisher sees it (updates UI spec section 5.1).
const RELEASES = { to: "/releases", label: "Releases", icon: Rocket };

/// The frame around every screen: the icon rail, the banners, the screen, the toasts.
export function Layout() {
  useRuntimeEvents();
  useVoice();
  useActivity();
  const publisher = useQuery({ queryKey: keys.publisher, queryFn: commands.publisherStatus }).data;
  const isPublisher = publisher?.state.state === "publisher";
  // The health check after an update waits for this (updates UI spec section 4.4).
  useEffect(() => {
    commands.appRendered().catch(() => {});
  }, []);
  // Ctrl+K (Cmd+K on a Mac) opens the search box from anywhere, and closes it again.
  const [searching, setSearching] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isSearchShortcut(e)) return;
      e.preventDefault();
      setSearching((open) => !open);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  }).data;
  // What the dots have for the person, read or not (needs-you spec section 5).
  const unread = useQuery({ queryKey: keys.unreadTotal, queryFn: commands.unreadTotal }).data ?? 0;
  // The same count as the tray's (`inbox_count`), the open questions and unread notes included,
  // and what the person snoozed or marked done left out.
  const waiting = inboxWaiting(inbox, messages);
  // A pending approval or an open question needs the person: the counts take the needs colour
  // (deep-teal spec section 4).
  const needs =
    (inbox?.approvals.length ?? 0) > 0 ||
    (inbox?.app_reviews.length ?? 0) > 0 ||
    shownMessages(inbox, messages ?? []).some(isOpenQuestion);
  return (
    <div className="flex h-screen bg-background text-sm text-foreground">
      <nav
        aria-label="Main"
        className="flex w-[46px] shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-2 text-sidebar-foreground"
      >
        <AccountSwitcher />
        <button
          type="button"
          aria-label="Search (Ctrl+K)"
          title="Search (Ctrl+K)"
          aria-haspopup="dialog"
          onClick={() => setSearching(true)}
          className="flex size-9 items-center justify-center rounded-lg text-muted-foreground hover:bg-sidebar-accent hover:text-foreground"
        >
          <Search className="size-5" aria-hidden />
        </button>
        {(isPublisher ? [...LINKS.slice(0, -1), RELEASES, LINKS.at(-1)!] : LINKS).map(({ to, label, icon: Icon }) => {
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
      <SearchDialog open={searching} onOpenChange={setSearching} />
      <Toaster />
    </div>
  );
}
