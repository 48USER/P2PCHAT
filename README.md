# Rust based p2pChat

Peer-to-peer чат с поддержкой комнат, историей сообщений и возможность файлообмена, написанный на Rust.

## Технологии
- Rust
- [libp2p](https://github.com/libp2p/rust-libp2p) (gossipsub, mdns, noise, yamux)
- [Tokio](https://tokio.rs/)
- futures
- serde
- colored

## Инструкции по использованию
При запуске укажите путь к Вашему key-ring файлу и путь к Вашей публичной директории.
- `sub <room> <key>` – подключиться к комнате.
- `unsub <room>` – отключиться от комнаты.
- `pub <room>` – отправить сообщение в комнату.
- `set-nickname <peer_id> <nickname>` – установить ник для пира.
- `req file <peer_id> <file_name> [-m]` – запросить файл.
- `req menu <peer_id>` – запросить меню файлов.
- `res file <request_id> <file_path>` – ответить на запрос файла.
- `mailbox` – просмотреть входящие запросы.
- `ls accomplices|rooms|requests` – вывести список пиров, комнат или исходящих запросов.
- `close` – сохранить данные и закрыть клиент. 
