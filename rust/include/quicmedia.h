/*
 * quicmedia.h — C ABI for quicmedia-ffi (libquicmedia).
 *
 * This header is authoritative and hand-maintained; it can also be regenerated
 * with `cbindgen` (see rust/cbindgen.toml). All functions return int32_t status
 * (0 = success, non-zero = error code; see qm_error_message) unless noted, and
 * never unwind. No media data crosses this boundary — only lifecycle, config,
 * stats, and small data-channel messages.
 */
#ifndef QUICMEDIA_H
#define QUICMEDIA_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque session handle. */
typedef struct QmSession QmSession;

/* Connection state (matches ConnectionState). */
enum {
  QM_STATE_CONNECTING = 0,
  QM_STATE_CONNECTED = 1,
  QM_STATE_DISCONNECTED = 2,
  QM_STATE_FAILED = 3
};

/* role */
enum { QM_ROLE_SERVER = 0, QM_ROLE_CLIENT = 1 };
/* codec */
enum { QM_CODEC_OPUS = 0, QM_CODEC_PASSTHROUGH = 1 };
/* audio_backend */
enum { QM_BACKEND_AUTO = 0, QM_BACKEND_MINIAUDIO = 1, QM_BACKEND_NULL = 2 };
/* verification */
enum { QM_VERIFY_INSECURE = 0, QM_VERIFY_PINNED_CERT = 1, QM_VERIFY_PINNED_SHA256 = 2 };

typedef struct QmConfig {
  int32_t role;
  const char *bind_addr;        /* e.g. "0.0.0.0:0"  */
  const char *peer_addr;        /* nullable; required for client */
  const char *server_name;      /* SNI / cert name */
  uint32_t sample_rate;
  uint16_t channels;
  uint32_t frame_ms;
  uint32_t bitrate;
  int32_t codec;
  int32_t audio_backend;
  int32_t enable_audio;         /* bool */
  int32_t enable_data_channel;  /* bool */
  int32_t verification;
  const uint8_t *cert_der;      /* server cert (DER); required for server */
  size_t cert_der_len;
  const uint8_t *key_der;       /* PKCS#8 private key (DER); required for server */
  size_t key_der_len;
  const uint8_t *pin;           /* pinned cert DER, or 32-byte SHA-256, per verification */
  size_t pin_len;
} QmConfig;

typedef struct QmCallbacks {
  void *user;
  void (*on_state)(void *user, int32_t state);
  void (*on_data)(void *user, const uint8_t *data, size_t len);
  void (*on_error)(void *user, int32_t code, const char *message);
} QmCallbacks;

typedef struct QmStats {
  uint8_t state;
  uint64_t datagrams_sent;
  uint64_t datagrams_recv;
  uint64_t bytes_sent;
  uint64_t bytes_recv;
  uint64_t send_errors;
  uint64_t rtt_us;
  uint32_t jitter_depth;
  uint64_t jitter_played;
  uint64_t jitter_concealed;
  uint64_t jitter_late_drops;
} QmStats;

/* Initialize process-wide crypto defaults. Idempotent. */
int32_t qm_init(void);

/* Generate a self-signed certificate. On success *cert_out/*key_out are heap
 * buffers to release with qm_buffer_free. sans_csv may be NULL. */
int32_t qm_generate_self_signed(const char *sans_csv,
                                uint8_t **cert_out, size_t *cert_len,
                                uint8_t **key_out, size_t *key_len);

/* SHA-256 of a certificate DER into out[32]. */
int32_t qm_cert_sha256(const uint8_t *cert, size_t cert_len, uint8_t *out);

/* Start a session. On success *out is a handle freed with qm_session_stop. */
int32_t qm_session_start(const QmConfig *cfg, const QmCallbacks *cbs, QmSession **out);

/* Queue a message on the reliable data channel (non-blocking). */
int32_t qm_session_send_data(QmSession *session, const uint8_t *data, size_t len);

/* Fill *out with the current statistics snapshot. */
int32_t qm_session_get_stats(QmSession *session, QmStats *out);

/* Current connection state, or -1 if session is NULL. */
int32_t qm_session_state(QmSession *session);

/* Stop and free a session handle. NULL-safe. */
void qm_session_stop(QmSession *session);

/* Free a buffer returned by this library. */
void qm_buffer_free(uint8_t *ptr, size_t len);

/* Static, human-readable description for an error code. */
const char *qm_error_message(int32_t code);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* QUICMEDIA_H */
