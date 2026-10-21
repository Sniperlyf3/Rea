// quicmedia.hpp — header-only C++ RAII wrapper over the quicmedia C ABI.
//
// Mirrors the .NET binding: the engine owns many QUIC connections, each with
// several audio streams; it can capture, play back, and forward encoded packets
// natively (SFU). This wrapper only drives the control plane — no media crosses
// into C++.
//
//   #include <quicmedia/quicmedia.hpp>
//   quicmedia::AudioParams p;                 // defaults: Opus, 48k mono, 120ms, 8kbps
//   quicmedia::Engine::Callbacks cb;
//   cb.on_stream_opened = [&](auto conn, auto s, auto){ engine->bind_playback(conn, s); };
//   quicmedia::Engine engine(p, quicmedia::Backend::Auto, cb);
//
// Link against the native `quicmedia` library (see the CMake package).
#ifndef QUICMEDIA_HPP
#define QUICMEDIA_HPP

#include "quicmedia.h"

#include <cstdint>
#include <functional>
#include <stdexcept>
#include <string>
#include <vector>

namespace quicmedia {

enum class Codec : int32_t { Opus = QM_CODEC_OPUS, Passthrough = QM_CODEC_PASSTHROUGH };
enum class Backend : int32_t {
    Auto = QM_BACKEND_AUTO,
    Miniaudio = QM_BACKEND_MINIAUDIO,
    Null = QM_BACKEND_NULL,
};
enum class Verification : int32_t {
    Insecure = QM_VERIFY_INSECURE,
    PinnedCert = QM_VERIFY_PINNED_CERT,
    PinnedSha256 = QM_VERIFY_PINNED_SHA256,
};

using ConnId = uint64_t;
using StreamId = uint16_t;

/// Thrown when a native call returns a non-zero status.
class Error : public std::runtime_error {
public:
    explicit Error(int32_t code)
        : std::runtime_error(qm_error_message(code)), code_(code) {}
    int32_t code() const noexcept { return code_; }

private:
    int32_t code_;
};

inline void check(int32_t rc) {
    if (rc != 0) throw Error(rc);
}

/// Device/audio parameters shared by all streams on an engine.
struct AudioParams {
    uint32_t sample_rate = 48000;
    uint16_t channels = 1;
    uint32_t frame_ms = 120;
    uint32_t bitrate = 8000;
    Codec codec = Codec::Opus;
};

/// TLS material and peer verification policy for a connection.
struct Security {
    Verification verification = Verification::Insecure;
    std::vector<uint8_t> certificate; // DER; required for listen()
    std::vector<uint8_t> private_key; // PKCS#8 DER; required for listen()
    std::vector<uint8_t> pin;         // cert DER or 32-byte SHA-256, per verification
};

struct StreamParams {
    Codec codec;
    uint32_t sample_rate;
    uint16_t channels;
    uint32_t frame_ms;
    uint32_t bitrate;
};

struct ConnStats {
    uint64_t datagrams_sent;
    uint64_t datagrams_recv;
    uint64_t bytes_sent;
    uint64_t bytes_recv;
    uint64_t send_errors;
};

/// Generate a self-signed certificate: {certificate_der, pkcs8_private_key_der}.
inline std::pair<std::vector<uint8_t>, std::vector<uint8_t>>
generate_self_signed(const std::string &subject_alt_names_csv = "localhost") {
    uint8_t *cert = nullptr;
    uint8_t *key = nullptr;
    size_t cert_len = 0, key_len = 0;
    check(qm_generate_self_signed(subject_alt_names_csv.c_str(), &cert, &cert_len, &key, &key_len));
    std::vector<uint8_t> c(cert, cert + cert_len);
    std::vector<uint8_t> k(key, key + key_len);
    qm_buffer_free(cert, cert_len);
    qm_buffer_free(key, key_len);
    return {std::move(c), std::move(k)};
}

/// SHA-256 of a certificate DER (for Verification::PinnedSha256).
inline std::vector<uint8_t> cert_sha256(const std::vector<uint8_t> &cert_der) {
    std::vector<uint8_t> out(32);
    check(qm_cert_sha256(cert_der.data(), cert_der.size(), out.data()));
    return out;
}

/// A running media engine. Non-copyable, non-movable (the native layer holds a
/// pointer to this object for callbacks); keep it alive while in use.
class Engine {
public:
    struct Callbacks {
        std::function<void(ConnId, bool /*incoming*/)> on_connection;
        std::function<void(ConnId)> on_connection_closed;
        std::function<void(ConnId, StreamId, StreamParams)> on_stream_opened;
        std::function<void(ConnId, StreamId, bool /*active*/)> on_stream_active;
        std::function<void(ConnId, StreamId)> on_stream_closed;
        std::function<void(ConnId, const uint8_t *, size_t)> on_data;
        std::function<void(int32_t /*code*/, std::string)> on_error;
    };

    Engine(const AudioParams &params, Backend backend, Callbacks callbacks)
        : cb_(std::move(callbacks)) {
        QmAudioParams p{params.sample_rate, params.channels, params.frame_ms, params.bitrate,
                        static_cast<int32_t>(params.codec)};
        QmCallbacks c{};
        c.user = this;
        c.on_connection = &Engine::trampoline_connection;
        c.on_connection_closed = &Engine::trampoline_connection_closed;
        c.on_stream_opened = &Engine::trampoline_stream_opened;
        c.on_stream_active = &Engine::trampoline_stream_active;
        c.on_stream_closed = &Engine::trampoline_stream_closed;
        c.on_data = &Engine::trampoline_data;
        c.on_error = &Engine::trampoline_error;
        check(qm_engine_create(&p, static_cast<int32_t>(backend), &c, &engine_));
    }

    ~Engine() {
        if (engine_) qm_engine_destroy(engine_);
    }

    Engine(const Engine &) = delete;
    Engine &operator=(const Engine &) = delete;
    Engine(Engine &&) = delete;
    Engine &operator=(Engine &&) = delete;

    ConnId connect(const std::string &bind, const std::string &peer,
                   const std::string &server_name, const Security &sec, bool enable_data = false) {
        QmSecurity s = to_native(sec);
        uint64_t conn = 0;
        check(qm_engine_connect(engine_, bind.c_str(), peer.c_str(), server_name.c_str(), &s,
                                enable_data ? 1 : 0, &conn));
        return conn;
    }

    void listen(const std::string &bind, const Security &sec, bool enable_data = false) {
        QmSecurity s = to_native(sec);
        check(qm_engine_listen(engine_, bind.c_str(), &s, enable_data ? 1 : 0));
    }

    StreamId open_stream(ConnId conn, Codec codec, uint32_t bitrate, bool active) {
        uint16_t stream = 0;
        check(qm_engine_open_stream(engine_, conn, static_cast<int32_t>(codec), bitrate,
                                    active ? 1 : 0, &stream));
        return stream;
    }

    StreamId add_capture_stream(ConnId conn, Codec codec, uint32_t bitrate, bool active) {
        uint16_t stream = 0;
        check(qm_engine_add_capture_stream(engine_, conn, static_cast<int32_t>(codec), bitrate,
                                           active ? 1 : 0, &stream));
        return stream;
    }

    void bind_playback(ConnId conn, StreamId stream) {
        check(qm_engine_bind_playback(engine_, conn, stream));
    }

    void set_stream_active(ConnId conn, StreamId stream, bool active) {
        check(qm_engine_set_stream_active(engine_, conn, stream, active ? 1 : 0));
    }

    void close_stream(ConnId conn, StreamId stream) {
        check(qm_engine_close_stream(engine_, conn, stream));
    }

    void add_route(ConnId src_conn, StreamId src_stream, ConnId dst_conn, StreamId dst_stream) {
        check(qm_engine_add_route(engine_, src_conn, src_stream, dst_conn, dst_stream));
    }

    void remove_route(ConnId src_conn, StreamId src_stream, ConnId dst_conn, StreamId dst_stream) {
        check(qm_engine_remove_route(engine_, src_conn, src_stream, dst_conn, dst_stream));
    }

    void send_data(ConnId conn, const uint8_t *data, size_t len) {
        check(qm_engine_send_data(engine_, conn, data, len));
    }

    void close_connection(ConnId conn) {
        check(qm_engine_close_connection(engine_, conn));
    }

    ConnStats connection_stats(ConnId conn) {
        QmConnStats s{};
        check(qm_engine_connection_stats(engine_, conn, &s));
        return {s.datagrams_sent, s.datagrams_recv, s.bytes_sent, s.bytes_recv, s.send_errors};
    }

    static void init() { qm_init(); }

private:
    static QmSecurity to_native(const Security &sec) {
        QmSecurity s{};
        s.verification = static_cast<int32_t>(sec.verification);
        s.cert_der = sec.certificate.empty() ? nullptr : sec.certificate.data();
        s.cert_der_len = sec.certificate.size();
        s.key_der = sec.private_key.empty() ? nullptr : sec.private_key.data();
        s.key_der_len = sec.private_key.size();
        s.pin = sec.pin.empty() ? nullptr : sec.pin.data();
        s.pin_len = sec.pin.size();
        return s;
    }

    static Engine *self(void *user) { return static_cast<Engine *>(user); }

    static void trampoline_connection(void *u, uint64_t c, int32_t incoming) {
        if (auto &f = self(u)->cb_.on_connection) f(c, incoming != 0);
    }
    static void trampoline_connection_closed(void *u, uint64_t c) {
        if (auto &f = self(u)->cb_.on_connection_closed) f(c);
    }
    static void trampoline_stream_opened(void *u, uint64_t c, uint16_t s, int32_t codec,
                                         uint32_t sr, uint16_t ch, uint32_t fm, uint32_t br) {
        if (auto &f = self(u)->cb_.on_stream_opened)
            f(c, s, StreamParams{static_cast<Codec>(codec), sr, ch, fm, br});
    }
    static void trampoline_stream_active(void *u, uint64_t c, uint16_t s, int32_t active) {
        if (auto &f = self(u)->cb_.on_stream_active) f(c, s, active != 0);
    }
    static void trampoline_stream_closed(void *u, uint64_t c, uint16_t s) {
        if (auto &f = self(u)->cb_.on_stream_closed) f(c, s);
    }
    static void trampoline_data(void *u, uint64_t c, const uint8_t *d, size_t n) {
        if (auto &f = self(u)->cb_.on_data) f(c, d, n);
    }
    static void trampoline_error(void *u, int32_t code, const char *msg) {
        if (auto &f = self(u)->cb_.on_error) f(code, msg ? std::string(msg) : std::string());
    }

    QmEngine *engine_ = nullptr;
    Callbacks cb_;
};

} // namespace quicmedia

#endif // QUICMEDIA_HPP
