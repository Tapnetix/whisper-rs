#include "../whisper.cpp/src/whisper_dtw_median_filter.h"

#include <cassert>
#include <cstdint>
#include <vector>

static int expected_width(int requested, int64_t tokens) {
    if (tokens > requested) {
        return requested;
    }
    if (tokens == 1) {
        return 1;
    }
    const int largest_odd_below = (tokens % 2 == 0) ? (int) tokens - 1 : (int) tokens - 2;
    return largest_odd_below > 0 ? largest_odd_below : 1;
}

static std::vector<float> reference_filter(
        const std::vector<float>& input,
        int64_t heads,
        int64_t tokens,
        int64_t audio_tokens,
        int width) {
    std::vector<float> output(input.size());
    for (int64_t h = 0; h < heads; ++h) {
        for (int64_t t = 0; t < tokens; ++t) {
            for (int64_t a = 0; a < audio_tokens; ++a) {
                std::vector<float> values;
                for (int offset = -(width / 2); offset <= width / 2; ++offset) {
                    int64_t sample = a + offset;
                    if (sample < 0) {
                        sample = -sample;
                    }
                    if (sample >= audio_tokens) {
                        sample = 2 * (audio_tokens - 1) - sample;
                    }
                    values.push_back(input[(h * tokens + t) * audio_tokens + sample]);
                }
                for (size_t i = 0; i < values.size(); ++i) {
                    for (size_t j = i + 1; j < values.size(); ++j) {
                        if (values[j] < values[i]) {
                            const float temp = values[i];
                            values[i] = values[j];
                            values[j] = temp;
                        }
                    }
                }
                output[(h * tokens + t) * audio_tokens + a] = values[values.size() / 2];
            }
        }
    }
    return output;
}

static void check_axis(int64_t audio_tokens) {
    constexpr int64_t heads = 2;
    constexpr int64_t tokens = 3;
    std::vector<float> input(heads * tokens * audio_tokens);
    for (size_t i = 0; i < input.size(); ++i) {
        // Non-monotonic values exercise the median and reflected edge samples.
        input[i] = (float) ((i * 17 + i / 3 * 11) % 23);
    }
    std::vector<float> actual(input.size(), -999.0f);
    whisper_dtw::apply_median_filter(
        heads, tokens, audio_tokens, 7,
        [&](int64_t h, int64_t t, int64_t a) {
            return input[(h * tokens + t) * audio_tokens + a];
        },
        [&](int64_t h, int64_t t, int64_t a, float value) {
            actual[(h * tokens + t) * audio_tokens + a] = value;
        });

    const int width = expected_width(7, audio_tokens);
    const auto expected = reference_filter(input, heads, tokens, audio_tokens, width);
    assert(actual == expected);
}

int main() {
    assert(whisper_dtw::median_filter_input_supported(7, 2, 3, 1, 1, true));
    assert(whisper_dtw::median_filter_input_supported(7, 2, 3, 8, 1, true));
    assert(!whisper_dtw::median_filter_input_supported(6, 2, 3, 8, 1, true));
    assert(!whisper_dtw::median_filter_input_supported(7, 2, 3, 8, 2, true));
    assert(!whisper_dtw::median_filter_input_supported(7, 2, 3, 8, 1, false));
    assert(!whisper_dtw::median_filter_input_supported(7, 2, 1, 8, 1, true));
    assert(!whisper_dtw::median_filter_input_supported(7, 2, 3, 0, 1, true));

    const int64_t axes[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 3000};
    for (const auto axis : axes) {
        check_axis(axis);
    }

    // The full-width path remains exactly width 7 once eight tokens are present.
    assert(whisper_dtw::median_filter_width_for_audio_tokens(7, 8) == 7);
    assert(whisper_dtw::median_filter_width_for_audio_tokens(7, 3000) == 7);
    assert(whisper_dtw::median_filter_width_for_audio_tokens(7, 7) == 5);
    assert(whisper_dtw::median_filter_width_for_audio_tokens(7, 1) == 1);
}
