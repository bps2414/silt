# Divulgação em pt-BR (rascunho, nada publicado)

Para r/brdev, TabNews, grupos de Telegram/Discord de dev e X/Bluesky. Ajustar o tamanho a cada
lugar.

**Título:** Fiz o Silt: um analisador de disco para Windows que recupera espaço sem apagar o que você usa

**Texto:**

Todo analisador de disco termina em "essa pasta é grande, apague". Funciona para download velho,
mas o que mais enche o meu SSD é coisa que eu uso: jogos, toolchains, programas, `node_modules`.

O Windows tem uma resposta boa para isso desde o 10, e quase ninguém usa: ele consegue guardar
qualquer arquivo compactado e descompactar na hora da leitura (é o que o `compact /exe` faz). Os
programas nem percebem.

O Silt (https://github.com/bps2414/silt, grátis e open source) junta isso com um analisador:

- lê o índice do NTFS (a MFT) e varre o C: em cerca de 1 segundo;
- separa tudo em categorias numa barra em dither (apps, jogos, desenvolvimento, caches...);
- estima quanto cada pasta economizaria **antes** de mexer em qualquer coisa;
- **Shrink** compacta as pastas escolhidas, e **Restore** desfaz;
- **Clean** esvazia caches que voltam sozinhos (navegador, shader de GPU, npm, pip, Cargo...);
- **Trash** manda para a Lixeira; nada é apagado de vez sem passar por ela.

Números reais: toolchain do Rust 627 MB → 352 MB economizados; Git for Windows 396 MB → 196 MB;
um jogo do Xbox 5,1 GB → 2,9 GB economizados. Vídeo, imagem e arquivos compactados já vêm
comprimidos, e o Silt avisa que ali não tem o que ganhar.

Interface em português (segue o idioma do Windows), feita em Rust. Feedback é muito bem-vindo,
principalmente sobre as regras de limpeza (o README tem uma seção "É seguro?" listando
exatamente o que pode ser apagado).

**Imagem:** `docs/media/social.png`
