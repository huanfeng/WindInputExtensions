# 按键音音效包

每个子目录是一个音效包，设置里「音色」选到对应项时使用。文件命名规则见 `src/pack.rs`：
`default-1.wav`、`default-2.wav`……是普通键（随机挑一个），`space`、`enter`、`backspace`
等分类名或 `a`、`semicolon` 等键名的文件优先于 `default`。

## 自定义

在扩展目录下建 `packs/custom/`，放进 16 位 PCM 的 WAV（任意采样率、单/双声道都行），
设置里音色选「自定义」。最少只要一个 `default.wav`。改了文件后在设置里切一下音色即可重读。

## 来源与许可

| 包 | 文件 | 来源 | 许可 |
|---|---|---|---|
| keyboard | 全部 | unicaegames，[Keyboard Soundpack #1](https://opengameart.org/content/keyboard-soundpack-1-typing-and-single-keystrokes)（Cherry KC 1000） | CC0 |
| typewriter | `default-*`、`space-*`、`backspace`、`enter` 的回车段 | Work With Sounds / Konrad Gutkowski，[WWS Typewriter.ogg](https://commons.wikimedia.org/wiki/File:WWS_Typewriter.ogg)（Erika 5 打字机，1940） | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| typewriter | `enter` 的铃声段 | _stubb，[typewriter ding](https://freesound.org/people/_stubb/sounds/406243/) | CC0 |

加工：从原录音里切出单次击键（起音前留 2ms）、转 44.1kHz 单声道 16 位、末尾淡出、峰值归一；
`enter` 是铃声后接拉回滑架的一段。
