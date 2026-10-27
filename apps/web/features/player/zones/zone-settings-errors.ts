/** A zone request's error, in words, from the server's error code. */
export function errorText(error: unknown, fallback: string): string {
  const code = (
    error as { response?: { data?: { error?: string; message?: string } } }
  )?.response?.data?.error;
  const known: Record<string, string> = {
    set_private_on_the_device_itself:
      "A zone can only be made private from the device it runs on.",
    only_parson_apps_can_be_private:
      "Only Mujin Music apps can be private; speakers are shared by everyone.",
    crossfade_not_supported_here: "This zone cannot crossfade.",
    leveling_not_supported_here: "This zone cannot level volume.",
    comfort_outside_safety_limits:
      "The comfort limit must sit inside the safety limits.",
    invalid_safety_limits: "The safety minimum must be below the maximum.",
    zone_name_required: "Give the zone a name.",
    dsp_not_supported_here: "DSP is not available on this zone.",
    dsd_strategy_not_supported_here:
      "Only audio devices and bridges take a DSD playback strategy.",
    invalid_max_dsd_rate: "Pick one of the listed DSD rates.",
    exclusive_mode_is_for_audio_devices:
      "Exclusive mode is for audio devices of this computer.",
    volume_mode_not_supported_here: "This zone cannot use that volume mode.",
    max_format_not_supported_here:
      "This zone does not take a maximum sample rate.",
    volume_control_not_supported_here:
      "This zone's volume cannot be handed to Home Assistant.",
    zone_has_no_standby: "This zone has no standby.",
    fixed_volume:
      "This zone has fixed volume. Change it on the amplifier, or pick another volume mode in Device setup.",
  };
  return (code && known[code]) || (code ? code.replaceAll("_", " ") : fallback);
}
