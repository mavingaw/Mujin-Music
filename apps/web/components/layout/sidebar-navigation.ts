/** One link in the sidebar; `icon` names an icon in app-sidebar.tsx. */
export interface NavItem {
  href: string;
  label: string;
  icon: string;
}

export interface NavSection {
  title: string;
  items: NavItem[];
}

/**
 * Roon's grouping (Browse / My Library / Playlists) in Mujin's identity.
 * Library views are addressed by `?view=` so each one has its own link.
 */
export const sidebarSections: NavSection[] = [
  {
    title: "Browse",
    items: [
      { href: "/", label: "Home", icon: "home" },
      { href: "/genre", label: "Genres", icon: "genres" },
      { href: "/radio", label: "Live Radio", icon: "radio" },
      { href: "/streaming", label: "Streaming", icon: "streaming" },
      { href: "/listen-later", label: "Listen Later", icon: "listen-later" },
      { href: "/tags", label: "Tags", icon: "tags" },
      { href: "/history", label: "History", icon: "history" },
    ],
  },
  {
    title: "My Library",
    items: [
      { href: "/library", label: "Albums", icon: "albums" },
      { href: "/library?view=artists", label: "Artists", icon: "artists" },
      { href: "/library?view=songs", label: "Tracks", icon: "tracks" },
      {
        href: "/library?view=composers",
        label: "Composers",
        icon: "composers",
      },
      {
        href: "/library?view=compositions",
        label: "Compositions",
        icon: "compositions",
      },
    ],
  },
];

/** Where the app is, as far as the sidebar cares. */
export interface NavLocation {
  pathname: string;
  /** The `view` search parameter (library views). */
  view: string | null;
}

/**
 * Whether a sidebar item is the current page. Library views match on their
 * `view` parameter (Albums is the default view); other items match their
 * path and anything below it.
 */
export function isNavItemActive(item: NavItem, location: NavLocation): boolean {
  const [path, query = ""] = item.href.split("?");
  const itemView = new URLSearchParams(query).get("view");
  if (path === "/library") {
    if (location.pathname !== "/library") return false;
    const current = location.view ?? "albums";
    return (itemView ?? "albums") === current;
  }
  if (path === "/") return location.pathname === "/";
  return location.pathname === path || location.pathname.startsWith(`${path}/`);
}
