# tablet-screenshot

Take a screenshot from the connected Boox tablet via ADB and display it.

## Trigger

Use when the user says "take a screenshot of the tablet", "show me the tablet screen", or "screenshot from tablet".

## Steps

1. If the tablet has an **active eink review session** open, prefer the app's own capture —
   it composites the reader's ink over the page and does not depend on the framebuffer:
   ```bash
   eink-review screenshot <session-id> --out /tmp/tablet-screenshot.png
   ```
2. Otherwise capture through ADB:
   ```bash
   adb shell screencap -p /sdcard/screenshot.png && adb pull /sdcard/screenshot.png /tmp/tablet-screenshot.png && adb shell rm /sdcard/screenshot.png
   ```

3. Display it using the Read tool on `/tmp/tablet-screenshot.png`.

## Notes

- Requires a device connected via `adb devices`.
- The screenshot is saved temporarily at `/tmp/tablet-screenshot.png`.
- If no device is found, tell the user to connect the tablet via USB and enable USB debugging.
- `screencap` returns a **zero-byte file** while the eink-bridge app owns the raw drawing
  surface (observed on the Boox at 2026-10-30). Check the size before reading it; if it is
  empty, use the session capture above instead of retrying.
