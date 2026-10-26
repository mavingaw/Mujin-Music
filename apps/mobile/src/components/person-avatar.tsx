import { personHue, personInitials } from "@parson/music-sdk";
import { StyleSheet, Text, View, type ViewStyle } from "react-native";

import { Artwork } from "@/components/artwork";
import { apiUrl } from "@/lib/runtime";

/** A round photo of a person, or their initials when there is none. */
export function PersonAvatar({
  name,
  imageUrl,
  size,
  style,
}: {
  name: string;
  /** Server path (`/artists/{id}/image`) or a full URL. */
  imageUrl?: string | null;
  size: number;
  style?: ViewStyle;
}) {
  if (imageUrl) {
    const path = /^https?:\/\//.test(imageUrl) ? imageUrl : apiUrl(imageUrl);
    return <Artwork path={path} rounded={size / 2} size={size} style={style} />;
  }
  const hue = personHue(name);
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        styles.initials,
        {
          width: size,
          height: size,
          borderRadius: size / 2,
          backgroundColor: `hsl(${hue}, 30%, 28%)`,
        },
        style,
      ]}
    >
      <Text style={[styles.letters, { fontSize: Math.round(size * 0.36) }]}>
        {personInitials(name)}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  initials: {
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.15)",
  },
  letters: { color: "rgba(255,255,255,0.92)", fontWeight: "800" },
});
