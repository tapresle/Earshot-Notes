// C interface to macOS system audio capture, recording permissions, and the
// login item.
#ifndef GC_AUDIO_TAP_H
#define GC_AUDIO_TAP_H

#include <stddef.h>
#include <stdint.h>

// Receives interleaved float samples on a Core Audio thread, with the rate the
// capture device is running at now. The rate can change during a capture, for
// example when a Bluetooth headset switches profile. Must not block.
typedef void (*gc_audio_callback)(void *context, const float *interleaved, uint32_t frames,
                                  uint32_t channels, double sample_rate);

typedef struct {
    // The capture device's rate when capture started.
    double sample_rate;
    // The rate the tap's format reports, for diagnostics. It isn't the capture rate.
    double tap_sample_rate;
    uint32_t channels;
    // The output device being captured, as UTF-8, possibly truncated.
    char device_name[128];
} gc_audio_format;

// Starts capturing everything the system plays, except this process. Returns an
// opaque handle, or NULL with a message in `error` on failure.
void *gc_system_audio_start(gc_audio_callback callback, void *context, gc_audio_format *format,
                            char *error, size_t error_len);

// Stops capture. When this returns, the callback will not run again.
void gc_system_audio_stop(void *handle);

// Called on a private queue after the default output device changes, for
// example when AirPods connect. Must not block.
typedef void (*gc_device_change_callback)(void *context);

// Starts watching the default output device. Returns an opaque handle, or NULL
// with a message in `error` on failure.
void *gc_output_device_watch(gc_device_change_callback callback, void *context, char *error,
                             size_t error_len);

// Stops watching. When this returns, the callback will not run again.
void gc_output_device_unwatch(void *handle);

// Microphone permission, matching AVAuthorizationStatus:
// 0 not determined, 1 restricted, 2 denied, 3 authorized.
int32_t gc_mic_authorization_status(void);

// Shows the permission prompt if needed and blocks until it is answered.
// Returns 1 if access is granted. Do not call on the main thread.
int32_t gc_mic_request_access(void);

// System audio recording permission (kTCCServiceAudioCapture), through the
// private TCC framework loaded at runtime. Returns 0 not determined,
// 2 denied, 3 authorized, or -1 when the framework isn't available.
int32_t gc_system_audio_authorization_status(void);

// Shows the system audio prompt if needed and blocks until it is answered.
// Returns 1 if granted, 0 if denied, or -1 when the framework isn't
// available. Do not call on the main thread.
int32_t gc_system_audio_request_access(void);

// Sets the Dock icon from PNG or other image data. A bundled app gets its icon
// from the bundle; a development build, which isn't bundled, needs this.
// Returns 1 on success.
int32_t gc_set_app_icon(const uint8_t *image, size_t len);

// Hides the Dock icon the way menu bar apps do: hides the app first, so macOS
// hands focus to the next app with its usual app switch, then drops the Dock
// icon once the app is inactive. Dropping it while the app is frontmost, or
// within a second of showing it, makes macOS redraw the whole menu bar, so a
// hide that soon after a show waits. Call on the main thread.
void gc_hide_dock_icon(void);

// Shows the Dock icon again, cancelling a pending gc_hide_dock_icon. Call on
// the main thread.
void gc_show_dock_icon(void);

// Whether the app opens at login (SMAppService.mainAppService): 0 not
// registered, 1 enabled, 2 waiting for approval in System Settings, 3 not
// found (for example, an app that isn't bundled).
int32_t gc_login_item_status(void);

// Turns opening at login on or off. Returns 1 on success, or 0 with a message
// in `error`.
int32_t gc_login_item_set(int32_t enabled, char *error, size_t error_len);

// Returns 1 if macOS opened the app as a login item. Only meaningful while the
// app is finishing launching.
int32_t gc_launched_as_login_item(void);

#endif
