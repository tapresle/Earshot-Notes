// C interface to macOS system audio capture and microphone permission.
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

#endif
