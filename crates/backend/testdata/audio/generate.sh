#!/bin/sh
# Regenerates the audio fixtures used by the indexer's technical-property and
# credit tests (crates/backend/src/library/indexer.rs). Each clip is a quiet
# 0.3 second sine so the files stay small. Requires an ffmpeg build with
# libmp3lame, libvorbis and libopus.
set -eu
cd "$(dirname "$0")"

tone() {
  # $1 sample rate, $2 channel layout
  printf 'sine=frequency=440:sample_rate=%s:duration=0.3,volume=0.2,aformat=channel_layouts=%s' "$1" "$2"
}

enc() {
  # $1 sample rate, $2 channel layout, remaining arguments go to the encoder.
  rate=$1
  layout=$2
  shift 2
  ffmpeg -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact \
    -f lavfi -i "$(tone "$rate" "$layout")" \
    -metadata title=Fixture -metadata "artist=Fixture Artist" -metadata "album=Fixture Album" \
    "$@"
}

# FLAC 96 kHz / 24-bit stereo with Vorbis comment credits.
enc 96000 stereo -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 \
  -metadata "COMPOSER=Clara Composer" -metadata "CONDUCTOR=Carl Conductor" \
  -metadata "PRODUCER=Pat Producer" -metadata "PERFORMER=Piper Pianist (Piano)" \
  -metadata "REMIXER=Rémy Remixer" -metadata "LABEL=Fixture Records" \
  -metadata "ISRC=USRC17607839" \
  flac-96k-24bit-stereo.flac
# FLAC 44.1 kHz / 16-bit mono.
enc 44100 mono -c:a flac -sample_fmt s16 flac-44k-16bit-mono.flac
# MP3 44.1 kHz CBR 192 kbps stereo without tags (tests add ID3 frames).
enc 44100 stereo -c:a libmp3lame -b:a 192k -map_metadata -1 -id3v2_version 0 -write_xing 0 \
  mp3-44k-cbr192-stereo.mp3
# MP3 48 kHz VBR mono with a Xing header and no tags.
enc 48000 mono -c:a libmp3lame -q:a 4 -map_metadata -1 -id3v2_version 0 mp3-48k-vbr-mono.mp3
# ALAC 48 kHz / 24-bit stereo in MP4.
enc 48000 stereo -c:a alac -sample_fmt s32p -metadata "composer=Clara Composer" \
  alac-48k-24bit-stereo.m4a
# ALAC 44.1 kHz / 16-bit mono in MP4.
enc 44100 mono -c:a alac -sample_fmt s16p alac-44k-16bit-mono.m4a
# AAC LC 44.1 kHz 128 kbps stereo in MP4.
enc 44100 stereo -c:a aac -b:a 128k aac-44k-stereo.m4a
# Ogg Vorbis 44.1 kHz stereo.
enc 44100 stereo -c:a libvorbis -q:a 3 -metadata "COMPOSER=Clara Composer" vorbis-44k-stereo.ogg
# Opus from a 48 kHz mono source.
enc 48000 mono -c:a libopus -b:a 64k opus-48k-mono.opus
# WAV 22.05 kHz / 16-bit mono.
enc 22050 mono -c:a pcm_s16le -map_metadata -1 wav-22k-16bit-mono.wav
# AIFF 48 kHz / 24-bit stereo.
enc 48000 stereo -c:a pcm_s24be -map_metadata -1 aiff-48k-24bit-stereo.aiff

# Collector formats for the "play any format" tests. ffmpeg cannot write APE,
# TAK, DSF or DFF; the tests build minimal valid files for those in Rust.
tagged() {
  # Like enc, adding album artist, track, date, genre and composer tags.
  rate=$1
  layout=$2
  shift 2
  enc "$rate" "$layout" -metadata "album_artist=Fixture Artist" -metadata "track=3/9" \
    -metadata "date=1999" -metadata "genre=Jazz" -metadata "composer=Clara Composer" "$@"
}
# FLAC 48 kHz / 24-bit 5.1.
enc 48000 5.1 -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 flac-48k-24bit-5.1.flac
# WavPack 96 kHz / 24-bit stereo with APEv2 tags.
tagged 96000 stereo -c:a wavpack -sample_fmt s32p -bits_per_raw_sample 24 wavpack-96k-24bit-stereo.wv
# Windows Media Audio 2, 44.1 kHz stereo.
tagged 44100 stereo -c:a wmav2 -b:a 128k wma-44k-stereo.wma
# FLAC in Matroska, 48 kHz / 24-bit 5.1.
tagged 48000 5.1 -c:a flac -sample_fmt s32 mka-flac-48k-24bit-5.1.mka
# True Audio 44.1 kHz / 16-bit stereo.
tagged 44100 stereo -c:a tta -sample_fmt s16 tta-44k-16bit-stereo.tta
# ALAC in Core Audio Format, 44.1 kHz / 16-bit stereo.
enc 44100 stereo -c:a alac -sample_fmt s16p caf-alac-44k-16bit-stereo.caf
# Sony Wave64, 48 kHz / 24-bit stereo.
enc 48000 stereo -c:a pcm_s24le w64-48k-24bit-stereo.w64
# Raw AAC (ADTS), 44.1 kHz stereo.
enc 44100 stereo -c:a aac -b:a 128k aac-adts-44k-stereo.aac
# WAV with RIFF INFO tags and AIFF with an ID3 chunk.
tagged 44100 stereo -c:a pcm_s16le wav-tagged-44k-16bit-stereo.wav
tagged 44100 stereo -c:a pcm_s16be -write_id3v2 1 aiff-tagged-44k-16bit-stereo.aiff
