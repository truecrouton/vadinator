# vadinator

A voice-enabled client for chatting with llms. Includes `earshot` Voice Activity Detection (VAD) to cut down on non-vocal input and runs `whisper` locally for speech-to-text. Is intended to be *very* lightweight. Uses `v1/chat/completions/` API to work with different inference servers.

This project and this README is a work in progress.

Since it tries to detect break-in words like "Stop!" it may run slowly, depending on your machine, if not built with `--release`, especially if your inference server is on the same machine.

## Getting Started
You'll need to download the `piper` (e.g., `en_US-hfc_female-medium.onnx`) and `whisper` (e.g., `ggml-tiny.en.bin`) models and put these in the `./models` directory.

Then, setup your `vadinator.env` file. You can copy `vadinator.env.sample` as a starting point.

## Linux
If you have audio issues on linux you may need to check the default audio device. For example, running `aplay -L` should shoud a `default` device without `CARD=` and if it is then it's trying to use the hardware directly. On ubuntu, and probably other distros, you may need to install alsa libraries like `pipewire-alsa`, `alsa-utils`, and `libasound2-plugins`. Once `aplay-L` has a `default` device without `CARD=` in the name it *should* work.

For `espeak-ng` issues try installing `espeak-ng` or just `espeak-ng-data` and let `vadinator` know where the data is, e.g., `export PIPER_ESPEAKNG_DATA_DIRECTORY=[Directory *containing* the espeak-ng-data folder]`.

## Mac and Windows

The release builds *should* just work.

## Acknowledgements

This project is built on the shoulders of giants in the Rust audio and AI ecosystem:

* **[whisper-rs](https://github.com/tazz4843/whisper-rs)** - Rust bindings for `whisper.cpp`, providing our high-performance Speech-to-Text capabilities.
* **[Earshot](https://github.com/pykeio/earshot)** - Voice Activity Detection (VAD) that keeps the vadinator's ears sharp.
* **[thewh1teagle/piper-rs](https://github.com/thewh1teagle/piper-rs)** - A modern fork of Piper-rs for local Text-to-Speech.
* **[CPAL](https://github.com/RustAudio/cpal)** - Foundational low-level library for cross-platform audio input and output.
* **[Rodio](https://github.com/RustAudio/rodio)** - Audio playback engine that handles the smooth streaming of synthesized speech.
* **[Gemini (Google)](https://gemini.google.com)** - Assisted with Rust tutorial/debugging, low-level audio, and related questions.

Thanks to the **Tokio** and **Reqwest** contributors for the asynchronous backbone and networking that keep vadinator responsive.