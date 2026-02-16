#!/usr/bin/env bash
set -euo pipefail

FFMPEG_BIN="${FFMPEG_BIN:-ffmpeg}"
RTSP_URL="${RTSP_URL:-rtsp://127.0.0.1:8554/avena}"
CAMERA_DEVICE="${CAMERA_DEVICE:-0:none}"
FPS="${FPS:-20}"
VIDEO_SIZE="${VIDEO_SIZE:-1280x720}"
ENCODER="${ENCODER:-h264_videotoolbox}"

echo "Publishing MacBook camera to ${RTSP_URL}"
echo "Camera device: ${CAMERA_DEVICE}, fps=${FPS}, size=${VIDEO_SIZE}, encoder=${ENCODER}"

exec "${FFMPEG_BIN}" \
  -hide_banner \
  -loglevel warning \
  -f avfoundation \
  -framerate "${FPS}" \
  -video_size "${VIDEO_SIZE}" \
  -i "${CAMERA_DEVICE}" \
  -an \
  -c:v "${ENCODER}" \
  -pix_fmt yuv420p \
  -f rtsp \
  -rtsp_transport tcp \
  "${RTSP_URL}"
