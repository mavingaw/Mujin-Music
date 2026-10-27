import api, { isApiError } from "../core/http";
import { clearMediaToken, type AuthResponse } from "./auth";

/** What switching to a profile asks for. */
export type ProfileUnlock = "none" | "pin" | "password";

export type ProfileColor =
  | "violet"
  | "indigo"
  | "sky"
  | "teal"
  | "emerald"
  | "lime"
  | "amber"
  | "orange"
  | "rose"
  | "pink"
  | "slate";

/**
 * A person on this server. Every profile keeps its own history,
 * favourites, ratings, radio taste, recommendations and scrobbling.
 * `kind: "account"` signs in with a password; `kind: "profile"` is a
 * managed profile entered by switching from a signed-in device.
 */
export interface Profile {
  id: number;
  name: string;
  username: string;
  kind: "account" | "profile";
  role: string;
  color: ProfileColor;
  hasPin: boolean;
  quickSwitch: boolean;
  isCurrent: boolean;
  canEdit: boolean;
  canDelete: boolean;
  unlock: ProfileUnlock;
  listens: number;
  /** Connected scrobbling services ("lastfm", "listenbrainz"). */
  scrobbling: string[];
}

export interface ProfilesResponse {
  currentId: number;
  canCreate: boolean;
  colors: ProfileColor[];
  profiles: Profile[];
}

export interface CreateProfileInput {
  name: string;
  color?: ProfileColor;
  pin?: string;
}

export interface UpdateProfileInput {
  name?: string;
  color?: ProfileColor;
  quickSwitch?: boolean;
  /** A new PIN, or `null` to remove it. Leave out to keep it. */
  pin?: string | null;
}

export interface SwitchProfileInput {
  pin?: string;
  password?: string;
}

export interface SwitchProfileOptions {
  /** Ask for tokens in the body (native apps keep them in secure storage). */
  native?: boolean;
}

export async function getProfiles(): Promise<ProfilesResponse> {
  return (await api.get<ProfilesResponse>("/profiles")).data;
}

export async function createProfile(
  input: CreateProfileInput,
): Promise<Profile> {
  return (await api.post<Profile>("/profiles", input)).data;
}

export async function updateProfile(
  id: number,
  input: UpdateProfileInput,
): Promise<Profile> {
  return (await api.patch<Profile>(`/profiles/${id}`, input)).data;
}

export async function deleteProfile(id: number): Promise<void> {
  await api.delete(`/profiles/${id}`);
}

/**
 * Switches this device to another profile. Only this device's session
 * changes; playback keeps going. Resolves with the new session, or with
 * `status: false` and a message (wrong PIN, too many attempts).
 */
export async function switchProfile(
  id: number,
  input: SwitchProfileInput = {},
  options?: SwitchProfileOptions,
): Promise<AuthResponse> {
  try {
    const response = await api.post<AuthResponse>(
      `/profiles/${id}/switch`,
      input,
      options?.native
        ? { headers: { "X-Parson-Client": "native" } }
        : undefined,
    );
    // The media token belongs to the previous profile.
    clearMediaToken();
    return response.data;
  } catch (error) {
    const data = isApiError(error) ? error.response?.data : undefined;
    const message =
      typeof data === "object" && data && "message" in data
        ? String((data as { message: unknown }).message)
        : "Could not switch profiles.";
    const status = isApiError(error) ? error.response?.status : undefined;
    return {
      status: false,
      message,
      transient: !status || status >= 500,
    };
  }
}

/** A PIN is 4 to 8 digits. */
export function validProfilePin(value: string): boolean {
  return /^\d{4,8}$/.test(value);
}

/** One or two letters for an avatar: "Mia" → "M", "Ada Lovelace" → "AL". */
export function profileInitials(name: string): string {
  const words = name
    .trim()
    .split(/[\s._-]+/)
    .filter(Boolean);
  const [head, ...rest] = words;
  if (!head) return "?";
  const first = Array.from(head)[0] ?? "?";
  const tail = rest.at(-1);
  const last = tail ? (Array.from(tail)[0] ?? "") : "";
  return `${first}${last}`.toLocaleUpperCase();
}

/** Two stops of each profile colour, for avatar gradients on any client. */
export const PROFILE_GRADIENTS: Record<ProfileColor, [string, string]> = {
  violet: ["#a78bfa", "#6d28d9"],
  indigo: ["#818cf8", "#3730a3"],
  sky: ["#7dd3fc", "#0369a1"],
  teal: ["#5eead4", "#0f766e"],
  emerald: ["#6ee7b7", "#047857"],
  lime: ["#bef264", "#4d7c0f"],
  amber: ["#fcd34d", "#b45309"],
  orange: ["#fdba74", "#c2410c"],
  rose: ["#fda4af", "#be123c"],
  pink: ["#f9a8d4", "#be185d"],
  slate: ["#cbd5e1", "#334155"],
};

export function profileGradient(color: string): [string, string] {
  return PROFILE_GRADIENTS[color as ProfileColor] ?? PROFILE_GRADIENTS.slate;
}

/**
 * Profiles for the switcher: the current one first, then the others in
 * server order. Accounts hidden from the switcher are left out.
 */
export function switcherProfiles(response: ProfilesResponse): Profile[] {
  const visible = response.profiles.filter(
    (profile) => profile.quickSwitch || profile.isCurrent,
  );
  return [
    ...visible.filter((profile) => profile.isCurrent),
    ...visible.filter((profile) => !profile.isCurrent),
  ];
}
