# URLs HTTPS de desenvolvimento nomeadas (`suprnova dev:tls`)

Por padrão, o `suprnova serve` serve seu backend em um
`http://127.0.0.1:8765` bruto. Isso é bom para a maior
parte do desenvolvimento - mas alguns recursos do
navegador só funcionam via HTTPS em um host nomeado:

- **Passkeys / WebAuthn** - exigem um contexto seguro e uma
  origem estável.
- **Cookies `Secure`** e **`SameSite=None`** - só são
  definidos via HTTPS.
- **Service workers** - só registram via HTTPS (ou
  `localhost`).
- **URIs de redirecionamento OAuth/OIDC** - provedores
  costumam rejeitar hosts com IP/porta brutos.

O [portless](https://portless.sh) dá a cada app local uma
URL `https://<name>.localhost` estável, atrás de um único
proxy TLS na porta 443. O `suprnova dev:tls` conecta o
Suprnova ao portless e - a parte fácil de errar - confia
na CA local do portless em **todo repositório de
certificados do navegador na sua máquina**, sem sudo no
Linux.

> **Estritamente opt-in.** O portless nunca é obrigatório.
> O `suprnova serve` funciona sem o portless instalado.
> Você opta por ele ao fazer scaffold
> (`suprnova new <name> --with-portless`) ou adicionando um
> `portless.json` depois. Se você nunca rodar o `dev:tls`,
> nunca vai tocar no portless.

## Instale o portless

O portless é uma ferramenta Node:

```bash
npm install -g portless
```

Depois instale o proxy sempre ativo na porta 443, uma única
vez (esta é uma etapa de nível de sistema, que exige sudo e
pertence ao portless, não ao Suprnova):

```bash
portless service install
```

## Por projeto

Você tem duas formas de habilitar um projeto.

**Faça scaffold com a flag** - escreve o `portless.json` de
antemão:

```bash
suprnova new myapp --frontend svelte --with-portless
```

Isso gera um `portless.json` na raiz do projeto:

```json
{
  "name": "myapp",
  "appPort": 8765
}
```

`appPort` é o `SERVER_PORT` fixo do seu backend. Ele diz ao
portless que o app se vincula a uma porta conhecida (em vez
de o portless atribuir uma via `$PORT`), então a URL nomeada
roteia direto para ela.

**Adicione a um projeto existente** - escreva o mesmo
`portless.json` à mão (ou rode `portless alias myapp 8765`),
usando seu `SERVER_PORT`.

Depois, em **cada máquina** que vai rodar o app, faça o
registro único de confiança + rota:

```bash
cd myapp
suprnova dev:tls
```

Isso:

1. Verifica se o `portless` está no seu PATH.
2. Resolve o nome (`--name`, senão `[package].name` do
   `Cargo.toml`) e a porta (`--port`, senão `SERVER_PORT`,
   senão `8765`).
3. Registra a rota `myapp.localhost → 127.0.0.1:8765` (pule
   com `--no-alias`).
4. Confia na CA do portless nos repositórios de
   certificados dos seus navegadores.
5. Imprime os próximos passos.

Flags:

| Flag | Efeito |
|---|---|
| `--name <name>` | Sobrescreve o nome da URL. Padrão: nome do pacote no `Cargo.toml`. |
| `--port <port>` / `-p` | Sobrescreve a porta roteada. Padrão: `SERVER_PORT`, senão `8765`. |
| `--no-alias` | Só confia na CA; não toca na rota do portless. |
| `--yes` | Pula a confirmação antes de modificar seus repositórios de certificados. Ignorada quando o fingerprint da CA mudou desde a última execução - isso sempre pergunta. |

### Por que a etapa 4 pergunta primeiro

Confiar em uma CA significa que todo certificado que ela
assina é aceito pelo seu navegador, silenciosamente, para
todo site. Isso vale uma tecla pressionada
deliberadamente.

A CA é resolvida somente a partir do próprio estado do
portless, nunca de algo que o diretório do projeto possa
influenciar - um repositório clonado não consegue apontar o
`dev:tls` para uma CA de sua escolha. O comando imprime o
fingerprint que está prestes a confiar e espera sua
confirmação. Se o fingerprint for diferente do que foi
confiado anteriormente, ele pergunta mesmo sob `--yes`: uma
CA mudada é ou uma reinstalação do portless ou algo que você
quer examinar, e só você sabe dizer qual.

## Execute

```bash
suprnova serve
```

Abra `https://myapp.localhost`.

O backend se vincula à `8765` por padrão; o servidor dev do
Vite acompanha na `5765` via `http://localhost`. Uma página
servida a partir da origem HTTPS pode referenciar assets em
`http://localhost` porque navegadores tratam `localhost`
como um contexto seguro - isso **não** é bloqueado como
conteúdo misto.

> **Hot Module Reload via HTTPS é best-effort.** O websocket
> de HMR do Vite se conecta de volta ao servidor dev; se
> isso funciona de forma limpa via a origem HTTPS depende
> das suas versões de Vite/navegador. Se as atualizações ao
> vivo pararem de funcionar sob `https://`, aponte o Vite
> para uma origem de servidor dev HTTPS via a variável de
> ambiente `INERTIA_VITE_DEV_SERVER`. O carregamento de
> páginas e o resto do fluxo não são afetados.

## Vários apps

O portless é dono da 443 e multiplexa por subdomínio.
Registre cada app com seu próprio nome e porta:

```bash
suprnova dev:tls --name app-one --port 8765
suprnova dev:tls --name app-two --port 8766
```

Nunca vincule a 443 a partir de um app diretamente - isso é
trabalho do portless.

## Solução de problemas

**`ERR_CERT_AUTHORITY_INVALID` depois de rodar `dev:tls`.**
Seu navegador não foi totalmente reiniciado. Navegadores
leem seu repositório de certificados uma vez na
inicialização; recarregar a aba não basta. Digite
`chrome://restart` (ou saia completamente e abra de novo).

**`502 Bad Gateway`.** O proxy está de pé, mas seu backend
não está. Rode o `suprnova serve` no diretório do projeto.

**`portless trust` diz "A terminal is required to
authenticate".** Esse é o próprio comando do portless
precisando de um TTY de verdade para o `sudo`. O `suprnova
dev:tls` contorna isso completamente no Linux: ele instala a
CA direto nos repositórios NSS dos seus navegadores, que não
precisam de sudo.

**Um navegador Flatpak ainda não é confiável.** Navegadores
Flatpak guardam seu banco de dados NSS em
`~/.var/app/<id>/.pki/nssdb`. O `dev:tls` cobre esses casos -
rode-o de novo e reinicie completamente esse navegador.

**`certutil: command not found`.** Instale as ferramentas
NSS:

| Distro | Comando |
|---|---|
| Debian/Ubuntu | `sudo apt install libnss3-tools` |
| Fedora/RHEL | `sudo dnf install nss-tools` |
| Arch | `sudo pacman -S nss` |

**`portless CA not found at ~/.portless/ca.pem`.** O
portless gera sua CA quando o proxy roda pela primeira vez.
Inicie-o uma vez (`systemctl start portless`, ou `portless
proxy start`), depois rode o `suprnova dev:tls` de novo.

## Notas de plataforma

O caminho de NSS do navegador acima é o mecanismo do Linux.
No **macOS** e no **Windows**, navegadores leem o chaveiro
do SO / repositório de certificados, então o `dev:tls`
delega a confiança da CA ao `portless trust`, que tem como
alvo esses repositórios nativos.
