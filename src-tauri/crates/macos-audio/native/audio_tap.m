// System audio capture through a Core Audio process tap, and microphone permission.
//
// A global tap mixes the output of every process except this one. Taps can only
// be read through an aggregate device, so the tap is attached to a private
// aggregate device built on the default output device, and an IO proc reads it.

#import "audio_tap.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct {
    AudioObjectID tap;
    AudioObjectID aggregate;
    AudioDeviceIOProcID proc;
    dispatch_queue_t queue;
    gc_audio_callback callback;
    void *context;
    uint32_t channels;
    float *scratch;  // interleaving buffer for non-interleaved input
    uint32_t scratch_frames;
} gc_tap;

static void set_error(char *error, size_t len, NSString *message, OSStatus status) {
    if (!error || len == 0) return;
    snprintf(error, len, "%s (OSStatus %d)", message.UTF8String, (int)status);
}

static void destroy(gc_tap *t) {
    if (t->proc) {
        AudioDeviceStop(t->aggregate, t->proc);
        // Wait for any IO block already queued so the callback never runs after stop.
        if (t->queue) dispatch_sync(t->queue, ^{});
        AudioDeviceDestroyIOProcID(t->aggregate, t->proc);
    }
    if (t->aggregate != kAudioObjectUnknown) AudioHardwareDestroyAggregateDevice(t->aggregate);
    if (t->tap != kAudioObjectUnknown) {
        if (@available(macOS 14.2, *)) AudioHardwareDestroyProcessTap(t->tap);
    }
    free(t->scratch);
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

static void *start_tap(gc_audio_callback callback, void *context, gc_audio_format *format,
                       char *error, size_t error_len) API_AVAILABLE(macos(14.2)) {
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

        AudioObjectID output = kAudioObjectUnknown;
        status = get_property(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultSystemOutputDevice,
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
                    captured->callback(captured->context, buffer->mData, frames, channels);
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
                captured->callback(captured->context, captured->scratch, frames, channels);
            });
        if (status != noErr) {
            t->proc = NULL;
            set_error(error, error_len, @"Couldn't attach to the capture device", status);
            destroy(t);
            return NULL;
        }

        status = AudioDeviceStart(t->aggregate, t->proc);
        if (status != noErr) {
            set_error(error, error_len, @"Couldn't start system audio capture", status);
            destroy(t);
            return NULL;
        }

        format->sample_rate = asbd.mSampleRate;
        format->channels = t->channels;
    }
    return t;
}

void *gc_system_audio_start(gc_audio_callback callback, void *context, gc_audio_format *format,
                            char *error, size_t error_len) {
    if (@available(macOS 14.2, *)) {
        return start_tap(callback, context, format, error, error_len);
    }
    set_error(error, error_len, @"System audio capture needs macOS 14.2 or later", 0);
    return NULL;
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
