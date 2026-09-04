// ag-asr — M42d 本地语音识别 CLI（bionic-static，voiced 子进程调用）。
// 用法: ag-asr <wav>            模型固定 /var/models/asr（AG_ASR_DIR 覆盖）
// 出:   stdout 一行识别文本；失败 exit 1。
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "c-api.h"

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: ag-asr <wav>\n"); return 2; }
    const char *dir = getenv("AG_ASR_DIR");
    if (!dir || !*dir) dir = "/var/models/asr";
    char model[512], tokens[512];
    snprintf(model, sizeof model, "%s/model.int8.onnx", dir);
    snprintf(tokens, sizeof tokens, "%s/tokens.txt", dir);

    SherpaOnnxOfflineRecognizerConfig config;
    memset(&config, 0, sizeof config);
    config.feat_config.sample_rate = 16000;
    config.feat_config.feature_dim = 80;
    config.model_config.sense_voice.model = model;
    config.model_config.sense_voice.language = "auto";
    config.model_config.sense_voice.use_itn = 1;
    config.model_config.tokens = tokens;
    config.model_config.num_threads = 2;
    config.model_config.provider = "cpu";
    config.decoding_method = "greedy_search";

    const SherpaOnnxOfflineRecognizer *rec = SherpaOnnxCreateOfflineRecognizer(&config);
    if (!rec) { fprintf(stderr, "ag-asr: create recognizer failed\n"); return 1; }

    const SherpaOnnxWave *wave = SherpaOnnxReadWave(argv[1]);
    if (!wave) { fprintf(stderr, "ag-asr: read wave failed: %s\n", argv[1]); return 1; }

    const SherpaOnnxOfflineStream *s = SherpaOnnxCreateOfflineStream(rec);
    SherpaOnnxAcceptWaveformOffline(s, wave->sample_rate, wave->samples, wave->num_samples);
    SherpaOnnxDecodeOfflineStream(rec, s);
    const SherpaOnnxOfflineRecognizerResult *r = SherpaOnnxGetOfflineStreamResult(s);
    printf("%s\n", r && r->text ? r->text : "");
    if (r) SherpaOnnxDestroyOfflineRecognizerResult(r);
    SherpaOnnxDestroyOfflineStream(s);
    SherpaOnnxFreeWave(wave);
    SherpaOnnxDestroyOfflineRecognizer(rec);
    return 0;
}
