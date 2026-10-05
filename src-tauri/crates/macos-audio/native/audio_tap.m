// System audio capture through a Core Audio process tap, and microphone permission.
//
// A global tap mixes the output of every process except this one. Taps can only
// be read through an aggregate device, so the tap is attached to a private
// aggregate device built on the default output device, and an IO proc reads it.
//
// Samples arrive at the aggregate device's rate, not the rate in the tap's
// format. The device's rate can change while capturing (a Bluetooth headset
// drops from 48 kHz to 16 or 24 kHz when its microphone opens), so a listener
// tracks it and every buffer is reported with the rate it was captured at.

#import "audio_tap.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <unistd.h>

typedef struct {
    AudioObjectID tap;
    AudioObjectID aggregate;
    AudioDeviceIOProcID proc;
    dispatch_queue_t queue;
    gc_audio_callback callback;
    void *context;
    uint32_t channels;
    // Written by rate_listener and read by the IO block. Both run on `queue`,
    // which is serial, so they never race.
    double sample_rate;
    AudioObjectPropertyListenerBlock rate_listener;
    float *scratch;  // interleaving buffer for non-interleaved input
    uint32_t scratch_frames;
} gc_tap;

static const AudioObjectPropertyAddress kNominalRateAddress = {
    kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyElementMain};

static void set_error(char *error, size_t len, NSString *message, OSStatus status) {
    if (!error || len == 0) return;
    snprintf(error, len, "%s (OSStatus %d)", message.UTF8String, (int)status);
}

static void destroy(gc_tap *t) {
    if (t->rate_listener) {
        AudioObjectRemovePropertyListenerBlock(t->aggregate, &kNominalRateAddress, t->queue,
                                               t->rate_listener);
    }
    if (t->proc) AudioDeviceStop(t->aggregate, t->proc);
    // Wait for any IO or rate block already queued so the callback never runs after stop.
    if (t->queue) dispatch_sync(t->queue, ^{});
    if (t->proc) AudioDeviceDestroyIOProcID(t->aggregate, t->proc);
    if (t->aggregate != kAudioObjectUnknown) AudioHardwareDestroyAggregateDevice(t->aggregate);
    if (t->tap != kAudioObjectUnknown) AudioHardwareDestroyProcessTap(t->tap);
    free(t->scratch);
    t->rate_listener = nil;
    t->queue = nil;
    free(t);
}

static OSStatus get_property(AudioObjectID object, AudioObjectPropertySelector selector, UInt32 size,
                             void *out) {
    AudioObjectPropertyAddress address = {selector, kAudioObjectPropertyScopeGlobal,
                                          kAudioObjectPropertyElementMain};
    return AudioObjectGetPropertyData(object, &address, 0, NULL, &size, out);
}

static NSArray<NSNumber *> *own_process(void) {
    pid_t pid = getpid();
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyTranslatePIDToProcessObject,
                                          kAudioObjectPropertyScopeGlobal,
                                          kAudioObjectPropertyElementMain};
    AudioObjectID process = kAudioObjectUnknown;
    UInt32 size = sizeof(process);
    OSStatus status = AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(pid),
                                                 &pid, &size, &process);
    // A process that has never produced audio has no object yet; nothing to exclude.
    if (status != noErr || process == kAudioObjectUnknown) return @[];
    return @[ @(process) ];
}

void *gc_system_audio_start(gc_audio_callback callback, void *context, gc_audio_format *format,
                            char *error, size_t error_len) {
    gc_tap *t = calloc(1, sizeof(gc_tap));
    t->tap = kAudioObjectUnknown;
    t->aggregate = kAudioObjectUnknown;
    t->callback = callback;
    t->context = context;

    @autoreleasepool {
        CATapDescription *description =
            [[CATapDescription alloc] initStereoGlobalTapButExcludeProcesses:own_process()];
        description.UUID = [NSUUID UUID];
        description.name = @"Earshot system audio";
        description.privateTap = YES;
        description.muteBehavior = CATapUnmuted;

        OSStatus status = AudioHardwareCreateProcessTap(description, &t->tap);
        if (status != noErr) {
            set_error(error, error_len, @"Couldn't create the system audio tap", status);
            destroy(t);
            return NULL;
        }

        AudioStreamBasicDescription asbd = {0};
        status = get_property(t->tap, kAudioTapPropertyFormat, sizeof(asbd), &asbd);
        if (status != noErr) {
            set_error(error, error_len, @"Couldn't read the system audio format", status);
            destroy(t);
            return NULL;
        }
        if (asbd.mFormatID != kAudioFormatLinearPCM || !(asbd.mFormatFlags & kAudioFormatFlagIsFloat) ||
            asbd.mBitsPerChannel != 32 || asbd.mChannelsPerFrame == 0) {
            set_error(error, error_len, @"The system audio format isn't 32-bit float PCM", 0);
            destroy(t);
            return NULL;
        }
        t->channels = asbd.mChannelsPerFrame;

        // The default output device is where calls play. The default system output
        // device is only where alert sounds play, and can be a different device.
        AudioObjectID output = kAudioObjectUnknown;
        status = get_property(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice,
                              sizeof(output), &output);
        CFStringRef output_uid = NULL;
        if (status == noErr) {
            status = get_property(output, kAudioDevicePropertyDeviceUID, sizeof(output_uid), &output_uid);
        }
        if (status != noErr || !output_uid) {
            set_error(error, error_len, @"Couldn't find the default output device", status);
            destroy(t);
            return NULL;
        }
        NSString *outputUID = CFBridgingRelease(output_uid);
        CFStringRef output_name = NULL;
        if (get_property(output, kAudioObjectPropertyName, sizeof(output_name), &output_name) == noErr &&
            output_name) {
            NSString *name = CFBridgingRelease(output_name);
            strlcpy(format->device_name, name.UTF8String ?: "", sizeof(format->device_name));
        }

        NSDictionary *aggregate = @{
            @kAudioAggregateDeviceNameKey : @"Earshot capture",
            @kAudioAggregateDeviceUIDKey : [NSUUID UUID].UUIDString,
            @kAudioAggregateDeviceMainSubDeviceKey : outputUID,
            @kAudioAggregateDeviceIsPrivateKey : @YES,
            @kAudioAggregateDeviceIsStackedKey : @NO,
            @kAudioAggregateDeviceTapAutoStartKey : @YES,
            @kAudioAggregateDeviceSubDeviceListKey : @[ @{@kAudioSubDeviceUIDKey : outputUID} ],
            @kAudioAggregateDeviceTapListKey : @[ @{
                @kAudioSubTapDriftCompensationKey : @YES,
                @kAudioSubTapUIDKey : description.UUID.UUIDString,
            } ],
        };
        status = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)aggregate, &t->aggregate);
        if (status != noErr) {
            set_error(error, error_len, @"Couldn't create the capture device", status);
            destroy(t);
            return NULL;
        }

        // Fall back to the tap's rate only if the device won't report its own.
        Float64 device_rate = 0;
        if (get_property(t->aggregate, kAudioDevicePropertyNominalSampleRate, sizeof(device_rate),
                         &device_rate) != noErr ||
            device_rate <= 0) {
            device_rate = asbd.mSampleRate;
        }
        t->sample_rate = device_rate;

        t->queue = dispatch_queue_create("earshot.system-audio", DISPATCH_QUEUE_SERIAL);
        gc_tap *captured = t;
        status = AudioDeviceCreateIOProcIDWithBlock(
            &t->proc, t->aggregate, t->queue,
            ^(const AudioTimeStamp *now, const AudioBufferList *input, const AudioTimeStamp *input_time,
              AudioBufferList *output_data, const AudioTimeStamp *output_time) {
                (void)now; (void)input_time; (void)output_data; (void)output_time;
                if (!input || input->mNumberBuffers == 0) return;
                uint32_t channels = captured->channels;
                if (input->mNumberBuffers == 1) {
                    const AudioBuffer *buffer = &input->mBuffers[0];
                    uint32_t frames = buffer->mDataByteSize / (sizeof(float) * channels);
                    captured->callback(captured->context, buffer->mData, frames, channels,
                                       captured->sample_rate);
                    return;
                }
                // Non-interleaved: one buffer per channel.
                uint32_t frames = input->mBuffers[0].mDataByteSize / sizeof(float);
                if (frames > captured->scratch_frames) {
                    free(captured->scratch);
                    captured->scratch = malloc(sizeof(float) * frames * channels);
                    captured->scratch_frames = captured->scratch ? frames : 0;
                    if (!captured->scratch) return;
                }
                uint32_t buffers = input->mNumberBuffers < channels ? input->mNumberBuffers : channels;
                for (uint32_t c = 0; c < buffers; c++) {
                    const float *src = input->mBuffers[c].mData;
                    for (uint32_t f = 0; f < frames; f++) captured->scratch[f * channels + c] = src[f];
                }
                captured->callback(captured->context, captured->scratch, frames, channels,
                                   captured->sample_rate);
            });
        if (status != noErr) {
            t->proc = NULL;
            set_error(error, error_len, @"Couldn't attach to the capture device", status);
            destroy(t);
            return NULL;
        }

        t->rate_listener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) {
            (void)count; (void)addresses;
            Float64 rate = 0;
            if (get_property(captured->aggregate, kAudioDevicePropertyNominalSampleRate, sizeof(rate),
                             &rate) == noErr &&
                rate > 0) {
                captured->sample_rate = rate;
            }
        };
        status = AudioObjectAddPropertyListenerBlock(t->aggregate, &kNominalRateAddress, t->queue,
                                                     t->rate_listener);
        if (status != noErr) {
            t->rate_listener = nil;
            set_error(error, error_len, @"Couldn't watch the capture device's sample rate", status);
            destroy(t);
            return NULL;
        }

        status = AudioDeviceStart(t->aggregate, t->proc);
        if (status != noErr) {
            set_error(error, error_len, @"Couldn't start system audio capture", status);
            destroy(t);
            return NULL;
        }

        format->sample_rate = device_rate;
        format->tap_sample_rate = asbd.mSampleRate;
        format->channels = t->channels;
    }
    return t;
}

void gc_system_audio_stop(void *handle) {
    if (handle) destroy((gc_tap *)handle);
}

int32_t gc_mic_authorization_status(void) {
    return (int32_t)[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
}

int32_t gc_mic_request_access(void) {
    if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio] ==
        AVAuthorizationStatusAuthorized) {
        return 1;
    }
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block BOOL granted = NO;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                             completionHandler:^(BOOL ok) {
                                 granted = ok;
                                 dispatch_semaphore_signal(done);
                             }];
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    return granted ? 1 : 0;
}

// The private TCC framework has the only API that checks and requests system
// audio recording permission. It's resolved at runtime so a future macOS that
// changes it degrades to "unknown" instead of failing to launch.
static CFStringRef const kAudioCaptureService = CFSTR("kTCCServiceAudioCapture");
typedef long (*tcc_preflight_fn)(CFStringRef service, CFDictionaryRef options);
typedef void (*tcc_request_fn)(CFStringRef service, CFDictionaryRef options, void (^callback)(BOOL granted));

static void *tcc_symbol(const char *name) {
    static void *handle;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        handle = dlopen("/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC", RTLD_NOW);
    });
    return handle ? dlsym(handle, name) : NULL;
}

int32_t gc_system_audio_authorization_status(void) {
    tcc_preflight_fn preflight = (tcc_preflight_fn)tcc_symbol("TCCAccessPreflight");
    if (!preflight) return -1;
    switch (preflight(kAudioCaptureService, NULL)) {
        case 0: return 3;  // authorized
        case 1: return 2;  // denied
        default: return 0; // not determined
    }
}

int32_t gc_system_audio_request_access(void) {
    tcc_request_fn request = (tcc_request_fn)tcc_symbol("TCCAccessRequest");
    if (!request) return -1;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block BOOL granted = NO;
    request(kAudioCaptureService, NULL, ^(BOOL ok) {
        granted = ok;
        dispatch_semaphore_signal(done);
    });
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    return granted ? 1 : 0;
}
