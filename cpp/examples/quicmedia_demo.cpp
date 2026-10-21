// quicmedia_demo — minimal two-peer voice demo using the C++ wrapper.
//
//   quicmedia_demo server 0.0.0.0:5000
//   quicmedia_demo client 127.0.0.1:5000 <pin-base64>
//
// The server prints its certificate pin; the client pins it. Audio flows
// entirely in the native layer (mic -> Opus -> QUIC -> jitter -> speaker); this
// program only drives the control plane.

#include <quicmedia/quicmedia.hpp>
#include <quicmedia/easy.hpp>

#include <atomic>
#include <chrono>
#include <cstdio>
#include <string>
#include <thread>

namespace qm = quicmedia;

// Minimal, dependency-free base64 (encode/decode) for carrying the pin on the CLI.
static std::string b64_encode(const std::vector<uint8_t> &in) {
    static const char *t = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::string out;
    size_t i = 0;
    for (; i + 2 < in.size(); i += 3) {
        uint32_t n = (in[i] << 16) | (in[i + 1] << 8) | in[i + 2];
        out += t[(n >> 18) & 63];
        out += t[(n >> 12) & 63];
        out += t[(n >> 6) & 63];
        out += t[n & 63];
    }
    if (i < in.size()) {
        uint32_t n = in[i] << 16;
        if (i + 1 < in.size()) n |= in[i + 1] << 8;
        out += t[(n >> 18) & 63];
        out += t[(n >> 12) & 63];
        out += (i + 1 < in.size()) ? t[(n >> 6) & 63] : '=';
        out += '=';
    }
    return out;
}

static std::vector<uint8_t> b64_decode(const std::string &in) {
    auto val = [](char c) -> int {
        if (c >= 'A' && c <= 'Z') return c - 'A';
        if (c >= 'a' && c <= 'z') return c - 'a' + 26;
        if (c >= '0' && c <= '9') return c - '0' + 52;
        if (c == '+') return 62;
        if (c == '/') return 63;
        return -1;
    };
    std::vector<uint8_t> out;
    uint32_t buf = 0;
    int bits = 0;
    for (char c : in) {
        int v = val(c);
        if (v < 0) continue;
        buf = (buf << 6) | static_cast<uint32_t>(v);
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push_back(static_cast<uint8_t>((buf >> bits) & 0xFF));
        }
    }
    return out;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        std::fprintf(stderr,
                     "usage:\n"
                     "  %s host <bind-addr>              (easy VoiceAgent: host a call)\n"
                     "  %s join <peer-addr> <pin-b64>    (easy VoiceAgent: join + talk)\n"
                     "  %s server <bind-addr>            (low-level API)\n"
                     "  %s client <peer-addr> <pin-b64>  (low-level API)\n",
                     argv[0], argv[0], argv[0], argv[0]);
        return 1;
    }
    qm::Engine::init();

    const std::string mode = argv[1];
    const std::string addr = argv[2];

    // ---- easy layer: two-line host/join using VoiceAgent ----
    if (mode == "host") {
        quicmedia::easy::VoiceAgent agent;
        auto pin = agent.host(addr);
        std::printf("hosting on %s; pin:\n  %s\nPress Enter to quit\n", addr.c_str(),
                    b64_encode(pin).c_str());
        std::getchar();
        return 0;
    }
    if (mode == "join") {
        if (argc < 4) {
            std::fprintf(stderr, "join requires a pin argument\n");
            return 1;
        }
        quicmedia::easy::VoiceAgent agent;
        auto [conn, stream] = agent.join(addr, b64_decode(argv[3]));
        std::printf("joined (conn %llu, stream %u); talking. Press Enter to quit\n",
                    (unsigned long long)conn, (unsigned)stream);
        std::getchar();
        return 0;
    }

    qm::AudioParams params; // Opus, 48k mono, 120ms, 8kbps by default
    qm::Engine::Callbacks cb;
    cb.on_connection = [](qm::ConnId c, bool incoming) {
        std::printf("[conn %llu] %s\n", (unsigned long long)c, incoming ? "incoming" : "outgoing");
    };
    cb.on_error = [](int code, std::string msg) {
        std::printf("[error %d] %s\n", code, msg.c_str());
    };

    qm::Engine engine(params, qm::Backend::Auto, cb);

    if (mode == "server") {
        auto [cert, key] = qm::generate_self_signed("localhost");
        auto pin = qm::cert_sha256(cert);
        std::printf("server pin (base64 SHA-256):\n  %s\n", b64_encode(pin).c_str());

        qm::Security sec;
        sec.verification = qm::Verification::Insecure;
        sec.certificate = cert;
        sec.private_key = key;
        engine.listen(addr, sec);
        std::printf("listening on %s; press Enter to quit\n", addr.c_str());
        std::getchar();
    } else if (mode == "client") {
        if (argc < 4) {
            std::fprintf(stderr, "client requires a pin argument\n");
            return 1;
        }
        qm::Security sec;
        sec.verification = qm::Verification::PinnedSha256;
        sec.pin = b64_decode(argv[3]);

        auto conn = engine.connect("0.0.0.0:0", addr, "localhost", sec);
        auto stream = engine.add_capture_stream(conn, qm::Codec::Opus, params.bitrate, true);
        std::printf("connected (conn %llu, stream %u); sending mic audio. Press Enter to quit\n",
                    (unsigned long long)conn, (unsigned)stream);
        std::getchar();
    } else {
        std::fprintf(stderr, "unknown mode '%s'\n", mode.c_str());
        return 1;
    }
    return 0;
}
