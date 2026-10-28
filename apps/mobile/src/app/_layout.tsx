import {
  focusManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import {
  DarkTheme,
  DefaultTheme,
  Stack,
  ThemeProvider,
  useRouter,
  useSegments,
} from "expo-router";
import { useFonts } from "expo-font";
import * as SplashScreen from "expo-splash-screen";
import { StatusBar } from "expo-status-bar";
import { useEffect, useState } from "react";
import { AppState, Platform } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { layout, palette } from "@/constants/colors";
import { MiniPlayer } from "@/components/mini-player";
import { ActionDrawerProvider } from "@/components/action-drawer";
import { PlayerProvider } from "@/providers/player-provider";
import { SessionProvider, useSession } from "@/providers/session-provider";
import { ZonesProvider } from "@/providers/zones-provider";
import { AppearanceSync } from "@/providers/appearance-sync";

void SplashScreen.preventAutoHideAsync();

const baseTheme = palette.mode === "light" ? DefaultTheme : DarkTheme;
const theme = {
  ...baseTheme,
  colors: {
    ...baseTheme.colors,
    background: palette.background,
    card: palette.background,
    border: palette.border,
    primary: palette.text,
    text: palette.text,
  },
};

function PersistentMiniPlayer() {
  const segments = useSegments();
  const insets = useSafeAreaInsets();
  const session = useSession();
  const route = String(segments[segments.length - 1] ?? "");
  if (
    route === "player" ||
    route === "display" ||
    (session.phase !== "ready" && session.phase !== "offline")
  )
    return null;
  const inTabs = String(segments[0]) === "(tabs)";
  return (
    <MiniPlayer
      bottom={
        inTabs ? layout.tabBar + insets.bottom : Math.max(8, insets.bottom)
      }
    />
  );
}

function SessionNavigationGuard() {
  const router = useRouter();
  const segments = useSegments();
  const session = useSession();
  const topLevelRoute = String(segments[0] ?? "");
  const tabRoute = String((segments as readonly string[])[1] ?? "");
  const authenticated =
    session.phase === "ready" || session.phase === "offline";

  useEffect(() => {
    // The connect-link screen redirects itself once it has read the link.
    if (topLevelRoute === "connect") return;
    if (!authenticated && topLevelRoute) {
      router.replace("/");
    } else if (
      session.phase === "offline" &&
      topLevelRoute === "(tabs)" &&
      (tabRoute === "index" || tabRoute === "search" || !tabRoute)
    ) {
      router.replace("/(tabs)/library");
    } else if (authenticated && !topLevelRoute) {
      router.replace(
        session.phase === "offline" ? "/(tabs)/library" : "/(tabs)",
      );
    }
  }, [authenticated, router, session.phase, tabRoute, topLevelRoute]);

  return null;
}

function NativeQueryLifecycle() {
  useEffect(() => {
    if (Platform.OS === "web") return;
    focusManager.setFocused(AppState.currentState === "active");
    const subscription = AppState.addEventListener("change", (state) => {
      focusManager.setFocused(state === "active");
    });
    return () => {
      subscription.remove();
      focusManager.setFocused(undefined);
    };
  }, []);

  return null;
}

export default function RootLayout() {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: { queries: { staleTime: 30_000, retry: 1 } },
      }),
  );
  // Brand display face; the app shows with system type if it can't load.
  const [fontsLoaded, fontError] = useFonts({
    "Jost-Light": require("../../assets/fonts/jost/Jost-Light.ttf"),
    "Jost-Regular": require("../../assets/fonts/jost/Jost-Regular.ttf"),
    "Jost-Medium": require("../../assets/fonts/jost/Jost-Medium.ttf"),
  });
  const fontsSettled = fontsLoaded || Boolean(fontError);
  useEffect(() => {
    if (fontsSettled) void SplashScreen.hideAsync();
  }, [fontsSettled]);
  return (
    <GestureHandlerRootView
      style={{ flex: 1, backgroundColor: palette.background }}
    >
      <QueryClientProvider client={queryClient}>
        <NativeQueryLifecycle />
        <SessionProvider>
          <PlayerProvider>
            <ZonesProvider>
              <ActionDrawerProvider>
                <ThemeProvider value={theme}>
                  <StatusBar
                    style={palette.mode === "light" ? "dark" : "light"}
                  />
                  <AppearanceSync />
                  <Stack
                    screenOptions={{
                      headerShown: false,
                      contentStyle: { backgroundColor: palette.background },
                      animation: "slide_from_right",
                    }}
                  >
                    <Stack.Screen
                      name="index"
                      options={{ animation: "fade" }}
                    />
                    <Stack.Screen
                      name="(tabs)"
                      options={{ animation: "fade" }}
                    />
                    <Stack.Screen
                      name="connect"
                      options={{ animation: "fade" }}
                    />
                    <Stack.Screen
                      name="display"
                      options={{
                        presentation: "fullScreenModal",
                        animation: "fade",
                        orientation: "all",
                      }}
                    />
                    <Stack.Screen
                      name="player"
                      options={{
                        presentation: "fullScreenModal",
                        animation: "slide_from_bottom",
                        animationDuration: 180,
                        gestureEnabled: true,
                      }}
                    />
                  </Stack>
                  <SessionNavigationGuard />
                  <PersistentMiniPlayer />
                </ThemeProvider>
              </ActionDrawerProvider>
            </ZonesProvider>
          </PlayerProvider>
        </SessionProvider>
      </QueryClientProvider>
    </GestureHandlerRootView>
  );
}
