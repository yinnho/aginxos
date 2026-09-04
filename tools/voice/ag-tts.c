// ag-tts — M42d 本地语音合成 CLI（kokoro int8，bionic-static）。
// 用法: ag-tts <text> [out.wav]   一次性：合成一句退出，stdout 打印 wav 路径
//       ag-tts --serve [out.wav]  常驻（M42e）：stdin 一行文本一次合成，写
//                                  out.wav 后 stdout 回一行 "OK <rate> <n>"
//                                  / "ERR <why>"；EOF 退出。模型只加载一次
//                                  ——装载 ~3.8s 是短句延迟的大头（M42e 收据）。
// 模型 /var/models/tts/kokoro-int8-multi-lang-v1_1
//   AG_TTS_DIR / AG_TTS_SID(默认 8=中文女声) 可覆盖。
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "c-api.h"

int main(int argc, char **argv) {
    int serve = argc >= 2 && strcmp(argv[1], "--serve") == 0;
    if (argc < 2 || (serve && argc > 3) || (!serve && argc > 3)) {
        fprintf(stderr, "usage: ag-tts <text> [out.wav] | ag-tts --serve [out.wav]\n");
        return 2;
    }
    const char *dir = getenv("AG_TTS_DIR");
    if (!dir || !*dir) dir = "/var/models/tts/kokoro-int8-multi-lang-v1_1";
    const char *sid_s = getenv("AG_TTS_SID");
    int sid = sid_s && *sid_s ? atoi(sid_s) : 8;
    // 语音 UI 默认比 kokoro 原速利落一档（1.2×）；AG_TTS_SPEED 可覆盖。
    const char *speed_s = getenv("AG_TTS_SPEED");
    float speed = speed_s && *speed_s ? strtof(speed_s, NULL) : 1.2f;
    const char *out = serve
        ? (argc > 2 ? argv[2] : "/tmp/ag-tts-serve.wav")
        : (argc > 2 ? argv[2] : "/tmp/ag-tts-out.wav");

    char model[512], voices[512], tokens[512], espeak[512], dictd[512], lex[512], fsts[768];
    snprintf(model, sizeof model, "%s/model.int8.onnx", dir);
    snprintf(voices, sizeof voices, "%s/voices.bin", dir);
    snprintf(tokens, sizeof tokens, "%s/tokens.txt", dir);
    snprintf(espeak, sizeof espeak, "%s/espeak-ng-data", dir);
    snprintf(dictd, sizeof dictd, "%s/dict", dir);
    snprintf(lex, sizeof lex, "%s/lexicon-zh.txt", dir);
    snprintf(fsts, sizeof fsts, "%s/number-zh.fst,%s/date-zh.fst,%s/phone-zh.fst", dir, dir, dir);

    SherpaOnnxOfflineTtsConfig config;
    memset(&config, 0, sizeof config);
    config.model.num_threads = 2;
    config.model.kokoro.model = model;
    config.model.kokoro.voices = voices;
    config.model.kokoro.tokens = tokens;
    config.model.kokoro.data_dir = espeak;
    config.model.kokoro.dict_dir = dictd;
    config.model.kokoro.lexicon = lex;
    config.model.kokoro.length_scale = 1.0f;
    config.model.kokoro.lang = "zh";
    config.rule_fsts = fsts;
    config.max_num_sentences = 1;

    const SherpaOnnxOfflineTts *tts = SherpaOnnxCreateOfflineTts(&config);
    if (!tts) { fprintf(stderr, "ag-tts: create tts failed\n"); return 1; }

    SherpaOnnxGenerationConfig gen;
    memset(&gen, 0, sizeof gen);
    gen.sid = sid;
    gen.speed = speed;

    if (serve) {
        char line[4096];
        while (fgets(line, sizeof line, stdin)) {
            line[strcspn(line, "\r\n")] = 0;
            if (!line[0]) { printf("ERR empty\n"); fflush(stdout); continue; }
            const SherpaOnnxGeneratedAudio *audio =
                SherpaOnnxOfflineTtsGenerateWithConfig(tts, line, &gen, NULL, NULL);
            if (!audio || audio->n == 0) {
                if (audio) SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio);
                printf("ERR generate\n"); fflush(stdout); continue;
            }
            if (!SherpaOnnxWriteWave(audio->samples, audio->n, audio->sample_rate, out)) {
                printf("ERR write\n"); fflush(stdout);
            } else {
                printf("OK %d %d\n", audio->sample_rate, audio->n); fflush(stdout);
            }
            SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio);
        }
        SherpaOnnxDestroyOfflineTts(tts);
        return 0; // stdin EOF — 宿主退了，别变僵尸
    }

    const SherpaOnnxGeneratedAudio *audio =
        SherpaOnnxOfflineTtsGenerateWithConfig(tts, argv[1], &gen, NULL, NULL);
    if (!audio || audio->n == 0) { fprintf(stderr, "ag-tts: generate failed\n"); return 1; }

    if (!SherpaOnnxWriteWave(audio->samples, audio->n, audio->sample_rate, out)) {
        fprintf(stderr, "ag-tts: write failed: %s\n", out);
        return 1;
    }
    printf("%s\n", out);
    SherpaOnnxDestroyOfflineTtsGeneratedAudio(audio);
    SherpaOnnxDestroyOfflineTts(tts);
    return 0;
}
