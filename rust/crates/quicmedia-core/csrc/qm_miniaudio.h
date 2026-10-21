/*
 * qm_miniaudio.h — minimal C shim over miniaudio for quicmedia.
 *
 * Exposes a single full-duplex s16 device. The data callback runs on
 * miniaudio's audio thread and delivers interleaved 16-bit PCM for both
 * capture (input) and playback (output) at the requested sample rate/channels;
 * miniaudio performs any device-format/rate conversion internally.
 */
#ifndef QM_MINIAUDIO_H
#define QM_MINIAUDIO_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct qm_audio_device qm_audio_device;

/*
 * Audio-thread callback. `input` and `output` are interleaved s16 buffers of
 * `frame_count` PCM frames per channel (so frame_count * channels samples).
 * In duplex mode both are non-NULL.
 */
typedef void (*qm_audio_data_cb)(void *user,
                                 const int16_t *input,
                                 int16_t *output,
                                 uint32_t frame_count);

/* Open and start a duplex device. Returns 0 on success, negative on error. */
int qm_audio_open(uint32_t sample_rate,
                  uint16_t channels,
                  qm_audio_data_cb cb,
                  void *user,
                  qm_audio_device **out);

/* Stop and free a device. After this returns, no further callbacks fire. NULL-safe. */
void qm_audio_close(qm_audio_device *dev);

#ifdef __cplusplus
}
#endif

#endif /* QM_MINIAUDIO_H */
