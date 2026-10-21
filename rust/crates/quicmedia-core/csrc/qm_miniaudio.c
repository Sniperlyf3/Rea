/*
 * qm_miniaudio.c — miniaudio implementation + the quicmedia duplex shim.
 *
 * This is the single translation unit that compiles miniaudio. We disable the
 * high-level components we do not use (decoding/encoding/generation, resource
 * manager, node graph) to cut build time; raw device I/O stays enabled.
 */
#define MINIAUDIO_IMPLEMENTATION
#define MA_NO_DECODING
#define MA_NO_ENCODING
#define MA_NO_GENERATION
#define MA_NO_RESOURCE_MANAGER
#define MA_NO_NODE_GRAPH
#include "miniaudio.h"

#include "qm_miniaudio.h"
#include <stdlib.h>

struct qm_audio_device {
    ma_device device;
    qm_audio_data_cb cb;
    void *user;
};

static void qm_ma_data_callback(ma_device *pDevice,
                                void *pOutput,
                                const void *pInput,
                                ma_uint32 frameCount) {
    struct qm_audio_device *d = (struct qm_audio_device *)pDevice->pUserData;
    if (d != NULL && d->cb != NULL) {
        d->cb(d->user, (const int16_t *)pInput, (int16_t *)pOutput, frameCount);
    }
}

int qm_audio_open(uint32_t sample_rate,
                  uint16_t channels,
                  qm_audio_data_cb cb,
                  void *user,
                  qm_audio_device **out) {
    if (out == NULL || cb == NULL) {
        return -1;
    }
    *out = NULL;

    struct qm_audio_device *d = (struct qm_audio_device *)calloc(1, sizeof(*d));
    if (d == NULL) {
        return -2;
    }
    d->cb = cb;
    d->user = user;

    ma_device_config config = ma_device_config_init(ma_device_type_duplex);
    config.sampleRate = sample_rate;
    config.capture.format = ma_format_s16;
    config.capture.channels = channels;
    config.playback.format = ma_format_s16;
    config.playback.channels = channels;
    config.dataCallback = qm_ma_data_callback;
    config.pUserData = d;

    if (ma_device_init(NULL, &config, &d->device) != MA_SUCCESS) {
        free(d);
        return -3;
    }
    if (ma_device_start(&d->device) != MA_SUCCESS) {
        ma_device_uninit(&d->device);
        free(d);
        return -4;
    }

    *out = d;
    return 0;
}

void qm_audio_close(qm_audio_device *dev) {
    if (dev == NULL) {
        return;
    }
    ma_device_uninit(&dev->device); /* stops the audio thread; no callbacks after this */
    free(dev);
}
