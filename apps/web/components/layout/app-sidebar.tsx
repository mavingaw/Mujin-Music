"use client";

import {
  AudioLines,
  Clock,
  Disc3,
  Feather,
  Hash,
  Heart,
  History,
  Home,
  Import,
  Library,
  ListMusic,
  Music2,
  Plus,
  Radio,
  ScrollText,
  Search,
  Settings,
  Shapes,
  UserRound,
  type LucideIcon,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getPlaylists } from "@parson/music-sdk";
import ParsonBrandMark from "@/components/icons/parson-brand-mark";
import MujinLockup from "@/components/icons/mujin-lockup";
import CreatePlaylistDialog from "@/features/library/create-playlist-dialog";
import { useSession } from "@/features/account/session-provider";
import { cn } from "@/lib/cn";
import {
  isNavItemActive,
  type NavItem,
  sidebarSections,
} from "./sidebar-navigation";

const icons: Record<string, LucideIcon> = {
  home: Home,
  genres: Shapes,
  radio: Radio,
  streaming: AudioLines,
  "listen-later": Clock,
  tags: Hash,
  history: History,
  albums: Disc3,
  artists: UserRound,
  tracks: Music2,
  composers: Feather,
  compositions: ScrollText,
  imports: Import,
  settings: Settings,
};

const mobileNavigation = [
  { href: "/", label: "Home", icon: Home },
  { href: "/search", label: "Search", icon: Search },
  { href: "/library", label: "Library", icon: Library },
  { href: "/settings", label: "Settings", icon: Settings },
];

export default function AppSidebar() {
  return (
    <>
      <aside
        aria-label="Main navigation"
        className="fixed bottom-(--shell-footer) left-0 top-0 z-50 hidden w-(--shell-sidebar) flex-col border-r border-white/[0.06] bg-mujin-raised md:flex"
      >
        <Suspense fallback={<SidebarContent pathname="" search={null} />}>
          <SidebarWithLocation />
        </Suspense>
      </aside>
      <MobileTabBar />
    </>
  );
}

function SidebarWithLocation() {
  const pathname = usePathname();
  const searchParams = useSearchParams();
  return <SidebarContent pathname={pathname} search={searchParams} />;
}

function announceNavigation(href: string) {
  if (href === "/") {
    window.dispatchEvent(new Event("parson:navigate-home"));
  }
}

function SidebarContent({
  pathname,
  search,
}: {
  pathname: string;
  search: { get(name: string): string | null } | null;
}) {
  const view = search?.get("view") ?? null;
  const { session } = useSession();
  const playlists = useQuery({
    queryKey: ["playlists"],
    queryFn: getPlaylists,
    enabled: Boolean(session),
    staleTime: 60_000,
  });
  const [creating, setCreating] = useState(false);
  const location = { pathname, view };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Link
        href="/"
        aria-label="Mujin Music home"
        draggable={false}
        className="electron-titlebar-drag mx-auto mt-4 flex h-10 shrink-0 select-none items-center lg:mx-0 lg:mt-5 lg:px-6"
        onClick={() => announceNavigation("/")}
      >
        <ParsonBrandMark className="h-9 w-9 lg:hidden" />
        <MujinLockup className="hidden h-[22px] w-auto lg:block" />
      </Link>

      <nav className="app-shell-scroll mt-5 flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden pb-4 [scrollbar-gutter:auto]">
        {sidebarSections.map((section) => (
          <div className="mb-5" key={section.title}>
            <p className="mj-eyebrow mb-1.5 hidden px-6 text-[10.5px]! lg:block">
              {section.title}
            </p>
            <span
              aria-hidden="true"
              className="mx-auto mb-2 block h-px w-6 bg-white/8 lg:hidden"
            />
            <ul className="flex flex-col gap-0.5 px-3">
              {section.items.map((item) => (
                <li key={item.href}>
                  <SidebarLink
                    active={isNavItemActive(item, location)}
                    item={item}
                  />
                </li>
              ))}
            </ul>
          </div>
        ))}

        <div className="mb-2 hidden lg:block">
          <div className="mb-1.5 flex items-center justify-between pl-6 pr-4">
            <Link
              className="mj-eyebrow text-[10.5px]! transition-colors hover:text-mujin-text"
              href="/library?view=playlists"
            >
              Playlists
            </Link>
            <button
              aria-label="New playlist"
              className="grid h-6 w-6 place-items-center rounded-md text-mujin-text-tertiary transition-colors hover:bg-white/6 hover:text-mujin-text"
              onClick={() => setCreating(true)}
              title="New playlist"
              type="button"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </div>
          <ul className="flex flex-col gap-0.5 px-3">
            <li>
              <PlaylistLink
                active={
                  pathname === "/playlist" && search?.get("system") === "liked"
                }
                href="/playlist?system=liked"
                icon={<Heart className="h-3.5 w-3.5" />}
                name="Liked songs"
              />
            </li>
            {(playlists.data ?? []).slice(0, 40).map((playlist) => (
              <li key={playlist.id}>
                <PlaylistLink
                  active={
                    pathname === "/playlist" &&
                    search?.get("id") === String(playlist.id)
                  }
                  href={`/playlist?id=${playlist.id}`}
                  icon={<ListMusic className="h-3.5 w-3.5" />}
                  name={playlist.name}
                />
              </li>
            ))}
          </ul>
        </div>
      </nav>

      <div className="shrink-0 border-t border-white/[0.06] px-3 py-3">
        <SidebarLink
          active={isNavItemActive(
            { href: "/imports", label: "Imports", icon: "imports" },
            location,
          )}
          item={{ href: "/imports", label: "Imports", icon: "imports" }}
        />
        <SidebarLink
          active={pathname.startsWith("/settings")}
          item={{ href: "/settings", label: "Settings", icon: "settings" }}
        />
      </div>
      <CreatePlaylistDialog onOpenChange={setCreating} open={creating} />
    </div>
  );
}

function SidebarLink({ active, item }: { active: boolean; item: NavItem }) {
  const Icon = icons[item.icon] ?? Library;
  return (
    <Link
      aria-current={active ? "page" : undefined}
      aria-label={item.label}
      className={cn(
        "group relative flex h-10 items-center gap-3 rounded-lg px-3 text-[13.5px] font-medium transition-colors duration-150 max-lg:mx-auto max-lg:w-12 max-lg:justify-center max-lg:px-0 lg:h-9",
        active
          ? "bg-mujin-copper/[0.12] text-mujin-text"
          : "text-mujin-text-secondary hover:bg-white/[0.04] hover:text-mujin-text",
      )}
      href={item.href}
      onClick={() => announceNavigation(item.href)}
      prefetch
      title={item.label}
    >
      {active && (
        <span
          aria-hidden="true"
          className="absolute left-0 top-1/2 h-4 w-[3px] -translate-x-3 -translate-y-1/2 rounded-r-full bg-mujin-copper max-lg:-translate-x-0"
        />
      )}
      <Icon
        className={cn(
          "h-[18px] w-[18px] shrink-0 transition-colors",
          active
            ? "text-mujin-copper"
            : "text-mujin-text-tertiary group-hover:text-mujin-text-secondary",
        )}
        strokeWidth={1.7}
      />
      <span className="truncate max-lg:sr-only">{item.label}</span>
    </Link>
  );
}

function PlaylistLink({
  active,
  href,
  icon,
  name,
}: {
  active: boolean;
  href: string;
  icon: React.ReactNode;
  name: string;
}) {
  return (
    <Link
      className={cn(
        "flex h-8 items-center gap-3 rounded-lg px-3 text-[13px] transition-colors",
        active
          ? "bg-white/[0.06] text-mujin-text"
          : "text-mujin-text-secondary hover:bg-white/[0.04] hover:text-mujin-text",
      )}
      href={href}
      title={name}
    >
      <span className="text-mujin-text-tertiary">{icon}</span>
      <span className="truncate">{name}</span>
    </Link>
  );
}

function MobileTabBar() {
  const pathname = usePathname();
  const isActive = (href: string) =>
    pathname === href || (href !== "/" && pathname.startsWith(`${href}/`));
  return (
    <nav
      aria-label="Main navigation"
      className="fixed inset-x-0 bottom-0 z-50 flex h-(--shell-footer) items-stretch border-t border-white/[0.07] bg-mujin-base/92 pb-[env(safe-area-inset-bottom)] backdrop-blur-xl md:hidden"
    >
      {mobileNavigation.map(({ href, label, icon: Icon }) => {
        const active = isActive(href);
        return (
          <Link
            key={href}
            href={href}
            aria-current={active ? "page" : undefined}
            aria-label={label}
            onClick={() => announceNavigation(href)}
            className={cn(
              "relative flex min-w-0 flex-1 touch-manipulation flex-col items-center justify-center gap-1 text-[10.5px] font-medium tracking-wide transition-colors",
              active
                ? "text-mujin-text"
                : "text-mujin-text-tertiary active:text-mujin-text-secondary",
            )}
          >
            <Icon
              className={cn("h-[22px] w-[22px]", active && "text-mujin-copper")}
              strokeWidth={active ? 2 : 1.7}
            />
            <span>{label}</span>
          </Link>
        );
      })}
    </nav>
  );
}
