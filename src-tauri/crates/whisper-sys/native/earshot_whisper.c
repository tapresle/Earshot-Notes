#include "earshot_whisper.h"

#include <stdlib.h>
#include <string.h>

#include "whisper.h"

struct earshot_whisper {
    struct whisper_context *ctx;
};

// Beam search with five beams, as whisper-cli runs by default, which is what
// the #25 comparison measured.
static const int BEAM_SIZE = 5;
static const int THREADS = 4;

static _Thread_local char last_error[512];

// Keeps whisper.cpp and ggml quiet on stderr, and remembers the last error so
// a failed load can say why.
static void on_log(enum ggml_log_level level, const char *text, void *user_data) {
    (void)user_data;
    if (level != GGML_LOG_LEVEL_ERROR || text == NULL) {
        return;
    }
    strncpy(last_error, text, sizeof last_error - 1);
    last_error[sizeof last_error - 1] = '\0';
    size_t len = strlen(last_error);
    while (len > 0 && (last_error[len - 1] == '\n' || last_error[len - 1] == ' ')) {
        last_error[--len] = '\0';
    }
}

earshot_whisper *earshot_whisper_load(const char *model_path, int use_gpu) {
    whisper_log_set(on_log, NULL);
    last_error[0] = '\0';

    struct whisper_context_params params = whisper_context_default_params();
    params.use_gpu = use_gpu != 0;
    params.flash_attn = true;

    struct whisper_context *ctx = whisper_init_from_file_with_params(model_path, params);
    if (ctx == NULL) {
        return NULL;
    }
    earshot_whisper *whisper = malloc(sizeof *whisper);
    if (whisper == NULL) {
        whisper_free(ctx);
        strncpy(last_error, "out of memory", sizeof last_error - 1);
        return NULL;
    }
    whisper->ctx = ctx;
    return whisper;
}

int earshot_whisper_transcribe(earshot_whisper *whisper, const float *pcm,
                               int samples, char *out, int out_capacity) {
    last_error[0] = '\0';
    struct whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_BEAM_SEARCH);
    params.beam_search.beam_size = BEAM_SIZE;
    params.n_threads = THREADS;
    params.language = "en";
    params.translate = false;
    params.no_timestamps = true;
    params.print_progress = false;
    params.print_realtime = false;
    params.print_special = false;
    params.print_timestamps = false;

    if (whisper_full(whisper->ctx, params, pcm, samples) != 0) {
        if (last_error[0] == '\0') {
            strncpy(last_error, "whisper_full failed", sizeof last_error - 1);
        }
        return -1;
    }

    size_t total = 0;
    int segments = whisper_full_n_segments(whisper->ctx);
    for (int i = 0; i < segments; i++) {
        const char *text = whisper_full_get_segment_text(whisper->ctx, i);
        size_t len = strlen(text);
        if (out != NULL && total + len < (size_t)out_capacity) {
            memcpy(out + total, text, len);
        }
        total += len;
    }
    if (out != NULL && out_capacity > 0) {
        out[total < (size_t)out_capacity ? total : (size_t)out_capacity - 1] = '\0';
    }
    return total > 0x7fffffff ? 0x7fffffff : (int)total;
}

void earshot_whisper_free(earshot_whisper *whisper) {
    if (whisper == NULL) {
        return;
    }
    whisper_free(whisper->ctx);
    free(whisper);
}

const char *earshot_whisper_last_error(void) {
    return last_error[0] == '\0' ? NULL : last_error;
}
