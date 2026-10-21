// easy.hpp — a thin convenience layer over quicmedia::Engine for the common
// voice roles, so typical use is two lines:
//
//   quicmedia::easy::VoiceAgent agent;
//   auto pin = agent.host("0.0.0.0:5000");          // host a call; share the pin
//   // ... on another machine ...
//   agent.join("host:5000", pin);                   // join and start talking
//
// VoiceAgent plays back every inbound stream automatically (you always hear the
// other side) and hosts/joins with a self-signed cert + SHA-256 pin. Reach the
// full API any time via agent.engine().
#ifndef QUICMEDIA_EASY_HPP
#define QUICMEDIA_EASY_HPP

#include "quicmedia.hpp"

#include <memory>
#include <utility>

namespace quicmedia::easy {

/// A symmetric voice agent: auto-plays inbound streams and offers one-call
/// host/join helpers. Non-copyable/movable (the engine holds a pointer to it).
class VoiceAgent {
public:
    /// Raised on a native error (optional).
    std::function<void(int, std::string)> on_error;
    /// Raised when a new connection appears (optional).
    std::function<void(ConnId, bool /*incoming*/)> on_connection;

    explicit VoiceAgent(AudioParams params = {}, Backend backend = Backend::Auto)
        : params_(params) {
        Engine::Callbacks cb;
        cb.on_stream_opened = [this](ConnId c, StreamId s, StreamParams) {
            engine_->bind_playback(c, s); // hear every inbound stream by default
        };
        cb.on_connection = [this](ConnId c, bool incoming) {
            if (on_connection) on_connection(c, incoming);
        };
        cb.on_error = [this](int code, std::string msg) {
            if (on_error) on_error(code, std::move(msg));
        };
        engine_ = std::make_unique<Engine>(params, backend, std::move(cb));
    }

    VoiceAgent(const VoiceAgent &) = delete;
    VoiceAgent &operator=(const VoiceAgent &) = delete;
    VoiceAgent(VoiceAgent &&) = delete;
    VoiceAgent &operator=(VoiceAgent &&) = delete;

    /// Host a call with a fresh self-signed certificate. Returns the pin to hand
    /// to joiners (e.g. base64-encode it out of band).
    std::vector<uint8_t> host(const std::string &bind) {
        auto [cert, key] = generate_self_signed("localhost");
        Security sec;
        sec.verification = Verification::Insecure;
        sec.certificate = cert;
        sec.private_key = key;
        engine_->listen(bind, sec);
        return cert_sha256(cert);
    }

    /// Join a call: connect pinning the host and start sending the microphone.
    /// Returns {connection, stream}.
    std::pair<ConnId, StreamId> join(const std::string &peer, const std::vector<uint8_t> &pin) {
        Security sec;
        sec.verification = Verification::PinnedSha256;
        sec.pin = pin;
        ConnId conn = engine_->connect("0.0.0.0:0", peer, "localhost", sec);
        StreamId stream = engine_->add_capture_stream(conn, params_.codec, params_.bitrate, true);
        return {conn, stream};
    }

    /// Mute/unmute a stream (the sender stops transmitting while muted).
    void set_muted(ConnId conn, StreamId stream, bool muted) {
        engine_->set_stream_active(conn, stream, !muted);
    }

    /// Escape hatch to the full API.
    Engine &engine() { return *engine_; }

private:
    std::unique_ptr<Engine> engine_;
    AudioParams params_;
};

} // namespace quicmedia::easy

#endif // QUICMEDIA_EASY_HPP
