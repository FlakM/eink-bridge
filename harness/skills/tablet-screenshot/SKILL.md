# tablet-screenshot

Take a screenshot from the connected Boox tablet via ADB and display it.

## Trigger

Use when the user says "take a screenshot of the tablet", "show me the tablet screen", or "screenshot from tablet".

## Steps

1. Capture and pull the screenshot:
   ```bash
   adb shell screencap -p /sdcard/screenshot.png && adb pull /sdcard/screenshot.png /tmp/tablet-screenshot.png && adb shell rm /sdcard/screenshot.png
   ```

2. Display it using the Read tool on `/tmp/tablet-screenshot.png`.

## Notes

- Requires a device connected via `adb devices`.
- The screenshot is saved temporarily at `/tmp/tablet-screenshot.png`.
- If no device is found, tell the user to connect the tablet via USB and enable USB debugging.
