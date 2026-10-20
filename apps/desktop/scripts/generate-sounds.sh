#!/usr/bin/env bash
# Synthesizes the app's sound effects from sine tones, so every sound we ship is our own
# work under the project licence. Requires ffmpeg with libopus and libmp3lame.
# Run from anywhere: scripts/generate-sounds.sh
set -euo pipefail

desktop="$(cd "$(dirname "$0")/.." && pwd)"
sounds="$desktop/features/custom-sounds/sounds"
mkdir -p "$sounds"

# A 5 ms linear attack keeps the tone onset from clicking.
attack='min(1,t/0.005)'

# Two rising notes (E6, A6).
new_mail="0.32*$attack*sin(2*PI*1318.5*t)*exp(-9*t)+0.32*gte(t,0.11)*min(1,(t-0.11)/0.005)*sin(2*PI*1760*(t-0.11))*exp(-7*(t-0.11))"
# A short falling blip when the send button is pressed.
hit_send="0.3*$attack*sin(2*PI*(900*t-1500*t*t))*exp(-30*t)"
# A quick upward sweep once the message has gone.
send="0.28*$attack*sin(2*PI*(500*t+1800*t*t))*exp(-10*t)"

render() {
  local expr="$1" duration="$2" out="$3"
  shift 3
  ffmpeg -hide_banner -loglevel error -y -f lavfi \
    -i "aevalsrc=exprs='${expr}':s=48000:d=${duration}" \
    -map_metadata -1 -fflags +bitexact -flags:a +bitexact "$@" "$out"
}

render "$new_mail" 0.7 "$sounds/new-mail.ogg" -c:a libopus -b:a 64k
render "$hit_send" 0.15 "$sounds/hit-send.ogg" -c:a libopus -b:a 64k
render "$send" 0.4 "$sounds/send.ogg" -c:a libopus -b:a 64k
# macOS plays notification sounds itself and only accepts MP3 from the bundle (see
# src/main/notification-ipc.ts).
render "$new_mail" 0.7 "$desktop/build/resources/mac/new-mail.mp3" -c:a libmp3lame -b:a 128k
