/*
 * quicmedia.h — C ABI for quicmedia-ffi (libquicmedia), engine edition.
 *
 * One engine owns many QUIC connections, each carrying several logical audio
 * streams (encoded packets in unreliable datagrams) plus a reliable control
 * channel. The engine can capture (mic -> encode), play back (decode ->
 * speaker), and/or forward encoded packets between connections natively (SFU).
 *
 * All functions return int32_t status (0 = success; see qm_error_message) unless
 * noted, and never unwind. No media data crosses this boundary.
 */
#ifndef QUICMEDIA_H
#define QUICMEDIA_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct QmEngine QmEngine;

/* codec */
enum { QM_CODEC_OPUS = 0, QM_CODEC_PASSTHROUGH = 1 };
/* audio backend */
enum { QM_BACKEND_AUTO = 0, QM_BACKEND_MINIAUDIO = 1, QM_BACKEND_NULL = 2 };
/* peer verification */
enum { QM_VERIFY_INSECURE = 0, QM_VERIFY_PINNED_CERT = 1, QM_VERIFY_PINNED_SHA256 = 2 };

typedef struct QmAudioParams {
  uint32_t sample_rate; /* e.g. 48000 */
  uint16_t channels;    /* 1 = mono */
  uint32_t frame_ms;    /* ptime, e.g. 120 */
  uint32_t bitrate;     /* Opus bits/sec, e.g. 8000 */
  int32_t codec;        /* QM_CODEC_* */
} QmAudioParams;

typedef struct QmSecurity {
  int32_t verification; /* QM_VERIFY_* */
  const uint8_t *cert_der;
  size_t cert_der_len;
  const uint8_t *key_der;
  size_t key_der_len;
  const uint8_t *pin; /* pinned cert DER, or 32-byte SHA-256, per verification */
  size_t pin_len;
} QmSecurity;

typedef struct QmConnStats {
  uint64_t datagrams_sent;
  uint64_t datagrams_recv;
  uint64_t bytes_sent;
  uint64_t bytes_recv;
  uint64_t send_errors;
} QmConnStats;

typedef struct QmCallbacks {
  void *user;
  void (*on_connection)(void *user, uint64_t conn, int32_t incoming);
  void (*on_connection_closed)(void *user, uint64_t conn);
  void (*on_stream_opened)(void *user, uint64_t conn, uint16_t stream, int32_t codec,
                           uint32_t sample_rate, uint16_t channels, uint32_t frame_ms,
                           uint32_t bitrate);
  void (*on_stream_active)(void *user, uint64_t conn, uint16_t stream, int32_t active);
  void (*on_stream_closed)(void *user, uint64_t conn, uint16_t stream);
  void (*on_data)(void *user, uint64_t conn, const uint8_t *data, size_t len);
  void (*on_error)(void *user, int32_t code, const char *message);
} QmCallbacks;

/* Initialize process-wide crypto defaults. Idempotent. */
int32_t qm_init(void);

/* Generate a self-signed certificate (DER cert + PKCS#8 key). Release buffers with
 * qm_buffer_free. sans_csv may be NULL. */
int32_t qm_generate_self_signed(const char *sans_csv, uint8_t **cert_out, size_t *cert_len,
                                uint8_t **key_out, size_t *key_len);

/* SHA-256 of a certificate DER into out[32]. */
int32_t qm_cert_sha256(const uint8_t *cert, size_t cert_len, uint8_t *out);

/* Create an engine. *out is freed with qm_engine_destroy. */
int32_t qm_engine_create(const QmAudioParams *params, int32_t backend, const QmCallbacks *cbs,
                         QmEngine **out);
void qm_engine_destroy(QmEngine *engine);

/* Dial a peer; *out_conn receives the connection id. */
int32_t qm_engine_connect(QmEngine *engine, const char *bind, const char *peer,
                          const char *server_name, const QmSecurity *sec, int32_t enable_data,
                          uint64_t *out_conn);

/* Accept incoming connections on bind. */
int32_t qm_engine_listen(QmEngine *engine, const char *bind, const QmSecurity *sec,
                         int32_t enable_data);

/* Open an outbound stream; *out_stream receives the stream id. */
int32_t qm_engine_open_stream(QmEngine *engine, uint64_t conn, int32_t codec, uint32_t bitrate,
                              int32_t active, uint16_t *out_stream);

/* Open an outbound stream fed by the local microphone (capture -> encode). */
int32_t qm_engine_add_capture_stream(QmEngine *engine, uint64_t conn, int32_t codec,
                                     uint32_t bitrate, int32_t active, uint16_t *out_stream);

/* Decode and play an inbound stream on the speaker. */
int32_t qm_engine_bind_playback(QmEngine *engine, uint64_t conn, uint16_t stream);

/* Set our desired active state for a stream (signals the peer). */
int32_t qm_engine_set_stream_active(QmEngine *engine, uint64_t conn, uint16_t stream,
                                    int32_t active);

/* Close a stream. */
int32_t qm_engine_close_stream(QmEngine *engine, uint64_t conn, uint16_t stream);

/* Route an inbound stream's encoded packets to an outbound stream (native fan-out). */
int32_t qm_engine_add_route(QmEngine *engine, uint64_t src_conn, uint16_t src_stream,
                            uint64_t dst_conn, uint16_t dst_stream);
int32_t qm_engine_remove_route(QmEngine *engine, uint64_t src_conn, uint16_t src_stream,
                               uint64_t dst_conn, uint16_t dst_stream);

/* Reliable data-channel message on a connection. */
int32_t qm_engine_send_data(QmEngine *engine, uint64_t conn, const uint8_t *data, size_t len);

/* Close a connection. */
int32_t qm_engine_close_connection(QmEngine *engine, uint64_t conn);

/* Fill *out with a connection's counters. */
int32_t qm_engine_connection_stats(QmEngine *engine, uint64_t conn, QmConnStats *out);

/* Free a buffer returned by this library. */
void qm_buffer_free(uint8_t *ptr, size_t len);

/* Static, human-readable description for an error code. */
const char *qm_error_message(int32_t code);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* QUICMEDIA_H */
