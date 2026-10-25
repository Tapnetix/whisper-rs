#pragma once

#include <algorithm>
#include <cstdint>
#include <vector>

namespace whisper_dtw {

inline bool median_filter_input_supported(
        int requested_width,
        int64_t n_heads,
        int64_t n_tokens,
        int64_t n_audio_tokens,
        int64_t n_fourth_axis,
        bool is_f32) {
    return requested_width > 0 && requested_width % 2 == 1
        && n_heads > 0 && n_tokens > 1 && n_audio_tokens > 0
        && n_fourth_axis == 1 && is_f32;
}

inline int median_filter_width_for_audio_tokens(int requested_width, int64_t audio_tokens) {
    if (requested_width < audio_tokens) {
        return requested_width;
    }
    const int largest_odd_width_below_tokens = (audio_tokens % 2)
        ? (int) audio_tokens - 2
        : (int) audio_tokens - 1;
    return largest_odd_width_below_tokens < 1 ? 1 : largest_odd_width_below_tokens;
}

template <typename ReadValue, typename WriteValue>
inline void apply_median_filter(
        int64_t n_heads,
        int64_t n_tokens,
        int64_t n_audio_tokens,
        int requested_width,
        ReadValue read_value,
        WriteValue write_value) {
    const int filter_width = median_filter_width_for_audio_tokens(requested_width, n_audio_tokens);
    std::vector<float> window;
    window.reserve(filter_width);
    for (int64_t head = 0; head < n_heads; ++head) {
        for (int64_t token = 0; token < n_tokens; ++token) {
            for (int64_t audio = 0; audio < n_audio_tokens; ++audio) {
                for (int64_t offset = -filter_width / 2; offset <= filter_width / 2; ++offset) {
                    int64_t index = audio + offset;
                    if (index < 0) {
                        index = -index;
                    } else if (index >= n_audio_tokens) {
                        index = 2 * (n_audio_tokens - 1) - index;
                    }
                    window.push_back(read_value(head, token, index));
                }
                std::sort(window.begin(), window.end());
                write_value(head, token, audio, window[window.size() / 2]);
                window.clear();
            }
        }
    }
}

} // namespace whisper_dtw
