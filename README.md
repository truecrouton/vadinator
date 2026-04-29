# vadinator

A voice-enabled client for chatting with llms. Includes `earshot` Voice Activity Detection (VAD) to cut down on non-vocal input and runs `whisper` locally for speech-to-text. Is intended to be *very* lightweight. Uses `v1/chat/completions/` API to work with different inference servers.

You'll need to download the `whisper` (e.g., `en_US-hfc_female-medium.onnx`) and `piper` (e.g., `base.en.bin`) models and put these in the `./models` directory.

This project and this README is a work in progress.

## Acknowledgements

This project is built on the shoulders of giants in the Rust audio and AI ecosystem:

* **[whisper-rs](https://github.com/tazz4843/whisper-rs)** - Rust bindings for `whisper.cpp`, providing our high-performance Speech-to-Text capabilities.
* **[Earshot](https://github.com/pykeio/earshot)** - Voice Activity Detection (VAD) that keeps the vadinator's ears sharp.
* **[thewh1teagle/piper-rs](https://github.com/thewh1teagle/piper-rs)** - A modern fork of Piper-rs for local Text-to-Speech.
* **[CPAL](https://github.com/RustAudio/cpal)** - Foundational low-level library for cross-platform audio input and output.
* **[Rodio](https://github.com/RustAudio/rodio)** - Audio playback engine that handles the smooth streaming of synthesized speech.
* **[Gemini (Google)](https://gemini.google.com)** - Assisted with Rust tutorial/debugging, low-level audio, and related questions.

Thanks to the **Tokio** and **Reqwest** contributors for the asynchronous backbone and networking that keep vadinator responsive.