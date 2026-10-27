"use client";

import { Input } from "@/components/ui/input";
import { ArrowRight, ChevronLeft, ChevronRight, Search } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import {
  FormEvent,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
  Suspense,
} from "react";
import AppSidebar from "@/components/layout/app-sidebar";
import RouteScrollRestoration from "@/components/layout/route-scroll-restoration";
import ParsonBrandMark from "@/components/icons/parson-brand-mark";
import { DesktopWindowControls } from "@/components/layout/desktop-window-controls";
import { ProfileSwitcher } from "@/features/profiles/profile-switcher";
import {
  clearCachedAlbumInfos,
  clearCachedArtistInfos,
  clearCachedSongInfos,
  getLibraryReadiness,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  invalidateCatalogRevisionQueries,
  libraryReadinessPollInterval,
  libraryReadinessShouldRefetch,
} from "@/features/library/library-readiness-state";

export default function AppShell({ children }: { children: ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const mainRef = useRef<HTMLElement>(null);
  const readiness = useQuery({
    queryKey: ["library", "readiness"],
    queryFn: getLibraryReadiness,
    refetchInterval: (query) => libraryReadinessPollInterval(query.state.data),
    refetchOnMount: (query) => libraryReadinessShouldRefetch(query.state.data),
    refetchOnReconnect: (query) =>
      libraryReadinessShouldRefetch(query.state.data),
    refetchOnWindowFocus: (query) =>
      libraryReadinessShouldRefetch(query.state.data),
  });

  useEffect(() => {
    if (!readiness.data) {
      return;
    }
    clearCachedAlbumInfos();
    clearCachedArtistInfos();
    clearCachedSongInfos();
    void invalidateCatalogRevisionQueries(queryClient);
  }, [queryClient, readiness.data?.catalog_revision]);

  useEffect(() => {
    const syncQueryFromLocation = () => {
      setQuery(
        pathname === "/search"
          ? (new URLSearchParams(window.location.search).get("q") ?? "")
          : "",
      );
    };
    syncQueryFromLocation();
    window.addEventListener("popstate", syncQueryFromLocation);
    return () => window.removeEventListener("popstate", syncQueryFromLocation);
  }, [pathname]);

  function navigateToSearch() {
    const value = query.trim();
    if (!value) {
      return;
    }
    const href = `/search?q=${encodeURIComponent(value)}`;
    window.dispatchEvent(
      new CustomEvent("parson:search-submitted", { detail: { href } }),
    );
    router.push(href);
  }

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    navigateToSearch();
  }

  function submitSearchFromKeyboard(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key !== "Enter" || event.nativeEvent.isComposing) {
      return;
    }
    event.preventDefault();
    navigateToSearch();
  }

  return (
    <div className="h-screen overflow-hidden bg-mujin-base text-mujin-text">
      <AppSidebar />

      <header className="electron-titlebar-drag fixed left-0 right-0 top-0 z-40 h-(--shell-header) border-b border-white/[0.05] bg-mujin-base/88 px-3 backdrop-blur-xl md:left-(--shell-sidebar) md:px-6">
        <div className="app-titlebar relative flex h-full items-center gap-3">
          <Link
            className="electron-titlebar-no-drag flex h-10 w-10 shrink-0 select-none items-center justify-center md:hidden"
            href="/"
            aria-label="Mujin Music home"
            draggable={false}
            onClick={() =>
              window.dispatchEvent(new Event("parson:navigate-home"))
            }
          >
            <ParsonBrandMark className="h-8 w-8" />
          </Link>
          <div className="electron-titlebar-no-drag hidden items-center gap-1 md:flex">
            <button
              aria-label="Back"
              className="grid h-9 w-9 place-items-center rounded-full text-mujin-text-secondary transition-colors hover:bg-white/[0.06] hover:text-mujin-text"
              onClick={() => router.back()}
              title="Back"
              type="button"
            >
              <ChevronLeft className="h-5 w-5" strokeWidth={1.8} />
            </button>
            <button
              aria-label="Forward"
              className="grid h-9 w-9 place-items-center rounded-full text-mujin-text-secondary transition-colors hover:bg-white/[0.06] hover:text-mujin-text"
              onClick={() => window.history.forward()}
              title="Forward"
              type="button"
            >
              <ChevronRight className="h-5 w-5" strokeWidth={1.8} />
            </button>
          </div>

          <form
            className="app-titlebar-search electron-titlebar-no-drag relative min-w-0 flex-1 md:max-w-120"
            onSubmit={submitSearch}
          >
            <Search className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-mujin-text-tertiary" />
            <Input
              aria-label="Search music"
              className="h-10 rounded-full border border-white/[0.08] bg-mujin-surface-1! py-2 pl-10 pr-11 text-sm text-mujin-text shadow-none placeholder:text-mujin-text-tertiary focus:placeholder:text-transparent focus-visible:border-mujin-copper/60 focus-visible:ring-0"
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={submitSearchFromKeyboard}
              placeholder="Search artists, albums, composers…"
              value={query}
            />
            <button
              aria-label="Search"
              className="absolute right-1 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full text-mujin-text-tertiary transition-colors hover:bg-white/10 hover:text-mujin-text disabled:opacity-0"
              disabled={!query.trim()}
              type="submit"
            >
              <ArrowRight className="h-4 w-4" />
            </button>
          </form>
          <div className="ml-auto flex h-full shrink-0 items-center">
            <ProfileSwitcher />
            <DesktopWindowControls inline />
          </div>
        </div>
      </header>

      <main
        ref={mainRef}
        className="app-content-shell app-shell-scroll overflow-x-hidden overflow-y-auto overscroll-contain bg-mujin-base"
      >
        <Suspense fallback={null}>
          <RouteScrollRestoration containerRef={mainRef} />
        </Suspense>
        {children}
      </main>
    </div>
  );
}
