// A small C interface over whisper.cpp, built with it into
// libearshot_whisper.dylib by scripts/build-whisper.sh. The library exports
// only these functions, so its copy of ggml can't clash with llama.cpp's.
//
// Keep this file and the bindings in src/lib.rs in sync.

#ifndef EARSHOT_WHISPER_H
#define EARSHOT_WHISPER_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct earshot_whisper earshot_whisper;

// Loads a ggml Whisper model. Returns NULL on failure; see
// earshot_whisper_last_error.
earshot_whisper *earshot_whisper_load(const char *model_path, int use_gpu);

// Transcribes 16 kHz mono float PCM as English, with the parameters the
// #25 comparison measured. Writes the text, NUL-terminated, to `out`.
// Returns the text's length in bytes, which may be `out_capacity` or more
// when `out` was too small to hold it, or -1 on failure.
int earshot_whisper_transcribe(earshot_whisper *whisper, const float *pcm,
                               int samples, char *out, int out_capacity);

void earshot_whisper_free(earshot_whisper *whisper);

// The last error whisper.cpp logged on this thread, or NULL.
const char *earshot_whisper_last_error(void);

#ifdef __cplusplus
}
#endif

#endif
