#include "../whisper.cpp/include/whisper.h"

extern "C" {

void whisper_rs_params_set_greedy_best_of(struct whisper_full_params * params, int best_of) {
    params->greedy.best_of = best_of;
}

void whisper_rs_params_set_beam_search(struct whisper_full_params * params, int beam_size, float patience) {
    params->beam_search.beam_size = beam_size;
    params->beam_search.patience = patience;
}

void whisper_rs_params_set_print_special(struct whisper_full_params * params, bool value) {
    params->print_special = value;
}

void whisper_rs_params_set_print_progress(struct whisper_full_params * params, bool value) {
    params->print_progress = value;
}

void whisper_rs_params_set_print_realtime(struct whisper_full_params * params, bool value) {
    params->print_realtime = value;
}

void whisper_rs_params_set_print_timestamps(struct whisper_full_params * params, bool value) {
    params->print_timestamps = value;
}

void whisper_rs_params_set_token_timestamps(struct whisper_full_params * params, bool value) {
    params->token_timestamps = value;
}

void whisper_rs_params_set_max_len(struct whisper_full_params * params, int value) {
    params->max_len = value;
}

void whisper_rs_params_set_language(struct whisper_full_params * params, const char * value) {
    params->language = value;
}

void whisper_rs_params_set_initial_prompt(struct whisper_full_params * params, const char * value) {
    params->initial_prompt = value;
}

}
