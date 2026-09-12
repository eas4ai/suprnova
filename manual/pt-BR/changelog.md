# Registro de mudanças

Um log legível, por versão, do que mudou no Suprnova. Cada seção de
versão é o registro de lançamento daquela versão. Uma versão é
lançada quando seu commit de versão e a tag `v<version>` correspondente
são enviados atomicamente. Mais recentes primeiro.

## 2.0.1 - 2026-09-12

### Corrigido

- **Uma URL `redis://` com índice de banco de dados seleciona esse banco
  em todo lugar.** O driver de filas e o hub de broadcast fanout carregam
  cada qual um produtor sea-streamer ao lado de suas conexões redis
  diretas, e o sea-streamer não lê o banco de dados lógico do caminho da
  URL - então uma fila apontada para `redis://host:6379/3` empurrava jobs
  para o banco `0` enquanto sua metade consumidora esperava no banco `3`,
  e o hub fanout operava direto sobre `0`. Os dois agora repassam o índice
  explicitamente, com a mesma regra de análise que o cliente redis aplica.
  Uma URL sem caminho continua selecionando o banco `0`, então nada muda
  para a forma comum.
- **`suprnova generate-types` termina sua saída com uma única quebra de
  linha.** A linha em branco que separa uma interface da seguinte também
  era escrita depois da última, então um projeto que impõe
  `git diff --check` falhava com `new blank line at EOF` em um arquivo que
  não conseguia corrigir à mão: a regeneração seguinte escrevia a linha em
  branco de volta. Só o final do arquivo muda; as linhas em branco entre as
  declarações ficam.

- **Um formulário de login ou de registro gerado pelo scaffold mostra seus
  erros de validação.** O controlador de autenticação gerado declarava uma
  prop `errors` em `LoginProps` e `RegisterProps` e a enviava como `None`.
  O framework semeia `errors` em toda página Inertia a partir da bolsa de
  validação gravada em flash na sessão, e uma prop explícita com o mesmo
  nome substitui essa semente, então a página recebia `errors: null` e
  credenciais inválidas voltavam para um formulário que não mostrava nada.
  Ambas as props agora estão vazias, as páginas Login e Register dos três
  frontends leem `useForm().errors`, e o `inertia-props.ts` do scaffold é
  a saída byte a byte do `suprnova generate-types` para os controladores
  do scaffold, então a primeira regeneração de um projeto não reescreve
  mais um arquivo que ninguém editou.

- **Uma aplicação gerada pelo scaffold serve seu frontend compilado.** O
  `routes.rs` gerado não registrava nenhum fallback de arquivos estáticos,
  então, assim que o servidor de desenvolvimento do Vite não estava
  rodando, toda URL `/assets/*` do shell HTML respondia `404`, inclusive
  dentro da imagem de produção que o `Dockerfile` do scaffold constrói.
  `routes.rs` agora termina com
  `fallback!(StaticFiles::public().handler())`: as rotas declaradas
  continuam vencendo, dotfiles como `public/assets/.vite/manifest.json` e
  travessia de caminho são recusados, e uma URL desconhecida continua
  renderizando a página `Error` do Inertia.

- **Uma aplicação gerada pelo scaffold verifica endereços de e-mail e
  redefine senhas.** O registro criava e logava o usuário sem enviar
  e-mail de verificação, e nenhuma rota oferecia verificação de e-mail ou
  recuperação de senha, embora o `User` gerado já implementasse
  `MustVerifyEmail` e `CanResetPassword` e a migração `auth_flow_tokens`
  já fosse incluída. O registro agora envia um link de verificação por
  `EmailVerification::send_link` e segue para `/verify-email`, que mostra
  o aviso, reenvia o link e o consome em `/verify-email/verify` apenas
  para o dono logado. `/forgot-password` envia um link de redefinição para
  um endereço verificado (um endereço desconhecido ou não verificado
  recebe a mesma resposta e nenhum e-mail) e `/reset-password` rotaciona a
  senha por `PasswordReset::complete_with_outcome`, recusando-se a
  terminar enquanto as outras sessões ou tokens de lembrar-me da conta não
  puderem ser revogados. Os starters Vue, React e Svelte trazem as páginas
  `ForgotPassword`, `ResetPassword` e `VerifyEmail` e um link "Forgot your
  password?" na página de login. Os links são construídos com `url::to`,
  então `APP_URL` precisa nomear o endereço em que os usuários chegam à
  aplicação, e o registro precisa de um transporte de e-mail funcionando:
  o `.env` que o scaffold escreve aponta `MAIL_DRIVER=smtp` para um
  capturador local na porta 1025 (o Mailpit que
  `suprnova docker:compose --with-mailpit` adiciona), ou defina
  `MAIL_DRIVER=log` para imprimir cada mensagem, link incluído, no log do
  servidor.

- **Um scaffold Svelte recém-gerado compila contra `@inertiajs/svelte`
  3.7.** O `main.ts` gerado declarava um `async setup`, e
  `@inertiajs/svelte` 3.7 tipa `setup` como retornando
  `SvelteRenderResult | void`, então um projeto gerado hoje (o template
  pede `^3.6.1`, que agora resolve para 3.7.1) falhava no `svelte-check`
  dentro do `npm run build` antes de uma única página ter sido escrita.
  `setup` agora é síncrono e encadeia o carregamento do catálogo de
  traduções na montagem, então a ordem que o template descreve não muda.

### Documentação

- **Os seis espelhos de idioma dos capítulos do 2.0.0 seguem as convenções
  do próprio manual.** Os capítulos traduzidos desde o 1.3.7 quebravam a
  prosa em japonês e chinês no meio da frase (1.653 quebras de linha que
  apareciam como espaços soltos), traduziam 59 termos de forma diferente do
  resto do seu idioma, traduziam 9 títulos repetidos de uma segunda maneira
  e deixavam 2 aspas sem par. Tudo isso agora está alinhado com os
  capítulos mais antigos, de modo que quem lê em qualquer idioma vê um só
  vocabulário e frases inteiras em todo o manual.

### Atualizando

- **Estas são correções do scaffold; uma aplicação gerada com 2.0.0
  mantém seus arquivos gerados.** Atualizar o crate do framework não muda
  nada em `src/` nem em `frontend/`. Para trazer as correções para um
  projeto existente, faça as mesmas edições à mão: remova o campo `errors`
  de `LoginProps` e `RegisterProps` e leia `useForm().errors` nas páginas;
  adicione `fallback!(StaticFiles::public().handler())` como última
  entrada do `routes!`; e copie os controladores `email_verification` e
  `password_reset`, suas rotas e as três páginas de auth de um projeto
  gerado com 2.0.1. A saída do `generate-types` só muda no final do
  arquivo.

## 2.0.0 - 2026-09-10

### Segurança

- **Um bearer token e uma sessão web agora são identidades separadas.** O
  cache de usuário com escopo de solicitação mantinha um único slot de
  usuário atual em que tudo escrevia e de que tudo lia, então uma sessão de
  navegador hidratada pelo `SessionMiddleware` satisfazia um `TokenGuard` na
  mesma solicitação, e um usuário resolvido para o guard web era devolvido a
  um código que havia pedido o guard de API. Credenciais bearer agora
  carregam a própria procedência nesse cache: o `BearerTokenMiddleware`
  registra o identificador que validou em um slot bearer só dele, o
  `TokenGuard` resolve e guarda ali o usuário completo, e só algo que chegou
  por uma credencial bearer consegue satisfazer um guard de token. Guards de
  sessão são guardados por nome de guard no mesmo lugar, então
  `Auth::guard("admin").user()` e `Auth::guard("web").user()` em uma mesma
  solicitação não resolvem mais para aquele dos dois que rodou primeiro. Os
  slots genéricos permanecem como uma visão de compatibilidade para a facade
  estática `Auth`, espelhados apenas a partir do guard padrão configurado,
  então `Auth::id()`, `Auth::check()` e `AuthMiddleware` se comportam
  exatamente como se comportavam para uma aplicação com um guard só e para
  uma solicitação somente de token que nunca instala uma sessão.

- **Um guard nomeado mantém o próprio principal, a própria credencial de
  remember-me e a própria revogação.** Fazer login através do
  `Auth::guard("admin")` escrevia o identificador na mesma chave de sessão
  que o guard padrão usa, então dois guards em uma aplicação compartilhavam
  um principal só e sair de qualquer um dos dois deslogava dos dois. Na
  sessão persistida cada guard agora é dono da própria entrada sob o mapa
  `_auth_guards`, e o cookie de remember-me carrega um portador etiquetado
  com o guard (`suprnova.remember.v1:` seguido do nome do guard e da
  credencial), então um cookie emitido para um guard não consegue
  reautenticar outro. Um cookie sem esse prefixo é lido como sendo do guard
  padrão, que é exatamente o que um cookie emitido por um release anterior
  é, então ninguém é deslogado pela atualização; um portador que nomeia uma
  versão que esta build não entende é recusado em vez de adivinhado. A
  revogação segue a mesma fronteira: sair, e o próprio caminho de rotação do
  middleware, aposentam exatamente o seletor que o guard dono emitiu, em vez
  de toda credencial que o usuário tem.

- **O `BasicAuthMiddleware` não aceita mais um slot de sessão obsoleto como
  prova.** A forma não stateless dele pulava o header `Authorization` sempre
  que o `Auth::check()` fosse verdadeiro, e o `Auth::check()` lê o slot de
  usuário atual com escopo de solicitação, que qualquer coisa mais cedo na
  cadeia poderia ter preenchido. Ele agora pede o principal de sessão
  persistido do guard com que foi configurado, e recusa de vez quando esse
  guard está ausente ou não é um guard stateful, então uma solicitação só é
  admitida sem credenciais quando uma linha de sessão real diz a quem ela
  pertence. A forma stateless sempre releu o header e está inalterada.

- **Credenciais de remember-me rotacionam como uma única substituição
  atômica.** A rotação removia a credencial aceita e inseria a sucessora
  dela como duas escritas: uma falha entre as duas consumia uma credencial
  válida e deixava o visitante sem forma de voltar, e um crash deixava as
  duas linhas vivas. O schema padrão agora substitui uma linha exata e ainda
  válida pela sucessora preparada dela em uma única operação, e um
  armazenamento que não consegue tornar atômicas a remoção condicional e a
  inserção da substituta falha fechado em vez de executá-las separadamente.
  A correspondência de seletor é exata, então uma credencial nunca é
  aposentada por uma colisão de prefixo, e uma transição de identidade
  síncrona dentro de um handler enfileira para revogação, no fim da
  solicitação, exatamente a credencial que ela invalidou, em vez de deixá-la
  viva.

- **A admissão de dois fatores é serializada, e uma escrita de bloqueio que
  falha derruba a solicitação junto.** Dois workers provando o mesmo código
  podiam ler o contador de tentativas cada um antes de qualquer um dos dois
  escrevê-lo, então um orçamento de força bruta admitia mais tentativas do
  que permitia. A verificação agora reserva capacidade de tentativa dentro
  da mesma operação serializada de armazenamento que admite a tentativa,
  devolvendo tanto a reserva quanto qualquer estado de falha finalizada
  observado nessa decisão única; uma cerimônia que não consegue fazer commit
  cancela a prova preparada dela, e o padrão do caminho de cancelamento
  falha fechado, para que uma implementação de verificador existente não
  consiga vazar capacidade reservada em silêncio. Uma escrita de contador de
  bloqueio que dá erro não é mais engolida: a tentativa é recusada. Promover
  uma sessão que está esperando por um segundo fator é uma única migração
  atômica, e a credencial bearer é suprimida antes de a chamada de
  armazenamento ser aguardada, então um timeout ou uma falha de backend
  nunca consegue deixar uma credencial presa a uma sessão de que o framework
  não fez commit.

- **A rotação de sessão falha fechada.** Rotacionar um id de sessão destrói
  a linha antiga e escreve uma nova. Um destroy que dava erro era registrado
  em log e ignorado, o que deixava a linha autenticada anterior passível de
  replay por quem tivesse o cookie antigo. O middleware agora retorna antes
  de escrever a substituta e antes de emitir o id novo, e expira a
  credencial antiga do navegador na saída. Em separado, o cookie que carrega
  uma sessão nova ou rotacionada é construído antes de a linha ser
  commitada: um cookie que não podia ser construído deixava no armazenamento
  uma sessão que navegador nenhum jamais conseguiria apresentar, e agora não
  deixa nada para trás.

- **Uma sessão que sobrevive ao próprio usuário para de autorizar, e revogar
  as sessões de um usuário alcança os guards nomeados.** O `AuthMiddleware`
  tratava a presença de um identificador persistido como prova de uma
  identidade, então um usuário excluído ou excluído por soft delete seguia
  passando por toda rota protegida até a sessão expirar. Ele agora resolve o
  usuário através do provider e limpa o slot obsoleto quando o provider não
  encontra nada; uma aplicação sem provider de usuários vinculado nenhum
  mantém o caminho rápido só de identificador, reconhecido pelo erro próprio
  dele e não por correspondência de texto de mensagem, e toda outra falha de
  provider é um erro, e não uma passagem. O `destroy_all_for_user` só casava
  com a coluna indexada `user_id`, que é nula para uma sessão autenticada
  apenas através de um guard nomeado, então essas sessões sobreviviam a um
  "sair de todos os lugares". Ele agora compara também as identidades de
  guard dentro de cada payload sobrevivente.

- **Cerimônias de autorização de dispositivo transicionam atomicamente e são
  validadas antes de serem consumidas.** Aprovar um código de dispositivo
  lia a cerimônia, depois a consumia, depois escrevia o grant, então uma
  substituta emitida sob o mesmo seletor entre a leitura e o consumo podia
  ser consumida no lugar do registro que de fato foi aprovado. O contrato de
  armazenamento agora vincula a transação que consome ao registro exato que
  uma leitura anterior observou, e transiciona uma cerimônia enquanto
  consome outra em um único passo atômico. Os dois métodos falham fechado
  por padrão, então uma implementação externa de armazenamento continua
  compatível em código-fonte sem receber em silêncio o comportamento mais
  fraco.

- **Um refresh de token de provider cujo desfecho é desconhecido é cercado
  por um fence, não repetido.** Um refresh de conta vinculada que começava e
  então perdia a resposta deixava um claim comum que expirava no horário
  previsto, então um segundo worker fazia refresh do mesmo grant e um dos
  dois resultados era descartado, levando junto um refresh token de uso
  único. Iniciar uma troca agora substitui o dono do claim por um dono de
  troca reservado, preservando o prazo original, para que os seguidores
  consigam distinguir uma troca viva de uma abandonada, e um armazenamento
  nunca deve reivindicar de volta uma linha cujo dono esteja nesse namespace
  reservado. Armazenamentos que não implementam o fence falham fechado.

- **O Web Push recusa enviar através de um transporte que possa seguir um
  redirecionamento.** O `EndpointPolicy::Strict` valida a URL de endpoint da
  assinatura, mas a validação só cobria a URL inicial: um cliente que segue
  redirecionamentos transforma um endpoint validado em um `3xx` para
  qualquer lugar, e o reqwest segue redirecionamentos por padrão. Um cliente
  que este crate constrói agora tem os redirecionamentos desabilitados à
  força, e o novo `WebPushClient::with_client_builder` aplica toda opção que
  o chamador queira (proxy, pinagem de TLS, timeouts) enquanto sobrescreve a
  política de redirecionamento. O `WebPushClient::with_client`, que recebe
  um cliente já construído cuja política de redirecionamento não pode ser
  inspecionada, agora recusa enviar sob o `Strict` com
  `WebPushError::UnconfinedRedirects`, antes da criptografia e antes de
  qualquer solicitação. O `WebPushClient::allow_unconfined_redirects` é o
  opt-out explícito para um chamador que sabe que o cliente dele é seguro.

- **Uma URL assinada fica vinculada ao caminho exato para o qual foi
  assinada.** A assinatura e a verificação as duas cortavam uma barra final
  antes de fazer o hash, o que fazia de `/orders/1` e `/orders/1/` uma
  assinatura só, e tornava um proxy que acrescenta uma barra indistinguível
  de um cliente que edita o caminho. O caminho agora recebe hash exatamente
  como aparece, então uma assinatura cobre um caminho e um caminho apenas.

- **Uma chave de cache não consegue mais endereçar um lock ou um índice de
  tags.** Registros de lock, de tag e de chave-tag do Redis viviam sob uma
  sentinela NUL depois do prefixo configurado, e uma chave fornecida pelo
  chamador que começasse com aquela sentinela caía no mesmo espaço, então um
  `Cache::forget` podia liberar um lock distribuído que outra pessoa estava
  segurando. Os valores e cada tipo de registro interno agora carregam
  componentes de namespace distintos antes da chave do chamador, então os
  dois espaços não conseguem se encontrar. Uma chave comum é armazenada
  exatamente onde era antes, então nada já em cache fica órfão. O driver de
  memória recebeu a mesma separação.

- **Webhooks de pagamento recusam o que não conseguem identificar e
  classificam duplicatas pelo banco de dados, e não por texto de mensagem.**
  Um evento de provider com um identificador ausente, não string ou só de
  espaços em branco entrava no namespace de idempotência compartilhado sob
  uma chave em branco, onde colidia com todo outro evento assim; agora ele é
  recusado antes de qualquer estado ser escrito. Uma duplicata concorrente é
  reconhecida a partir do código estruturado de violação de unicidade do
  SeaORM, e não a partir de um texto de erro legível por humanos que uma
  falha não relacionada também pode conter, e só uma releitura de um
  `processed_at` commitado é reconhecida como tal, então uma falha de
  escrita no espelho continua passível de retentativa. O timestamp da
  assinatura da Stripe é comparado com uma diferença absoluta sem sinal,
  então um valor `t=` extremo retorna um erro de assinatura em vez de
  estourar, e uma tolerância configurada negativa aceita apenas uma
  correspondência exata. A rota de webhook preserva o status tipado de um
  corpo que ela recusou, então um corpo acima do teto continua sendo um
  `413`, e não um `400` achatado.

- **Uma solicitação HTTP não idempotente não é mais reexecutada depois de um
  erro de transporte.** A política de retentativa já exigia o opt-in
  explícito `retry_non_idempotent` antes de reexecutar um `POST` ou um
  `PATCH` que respondesse `5xx`, mas o ramo de erro de transporte ao lado
  dele não conferia, então uma solicitação cuja conexão caiu depois de o
  servidor já a ter aceitado era enviada de novo. Os dois ramos agora
  aplicam a mesma regra.

- **Tempos de vida de sessão não conseguem estourar em uma expiração em
  massa.** O `SESSION_LIFETIME` e o `SESSION_REMEMBER_LIFETIME` são minutos
  multiplicados por sessenta e depois somados a um timestamp armazenado em
  aritmética de data que entra em panic no estouro, então um valor grande
  demais ou abortava o processo ou dava a volta para um prazo no passado que
  expirava toda sessão de uma vez. Os dois são limitados a
  `MAX_SESSION_LIFETIME_MINUTES` antes da multiplicação, o driver de banco
  de dados limita do mesmo jeito para uma configuração construída em código,
  e um corte de coleta de lixo que não pode ser representado é pulado em vez
  de enviado ao banco de dados, que é a diferença entre não coletar nada e
  coletar tudo.

- **Identidades de cache máquina-a-máquina são inequívocas.** A chave de
  cache do broker de tokens concatenava o provider, o cliente e o conjunto
  de escopos normalizado, então duas solicitações diferentes cujos
  componentes por acaso se juntassem na mesma string compartilhavam um token
  em cache só. A chave agora é um domínio versionado com componentes
  prefixados por comprimento, que combinação de entradas nenhuma consegue
  fazer colidir.

- **Um lease de idempotência é provado ainda em posse antes de o resultado
  dele ser reportado como cercado por um fence.** O lease se renovava
  periodicamente enquanto o corpo rodava, e um erro transitório de renovação
  era tratado como perda, enquanto um corpo que terminava entre duas
  renovações era reportado como cercado sem que ninguém perguntasse se o
  lock ainda estava lá. Um erro transitório de renovação agora é repetido e
  só desiste depois de várias falhas consecutivas, e uma renovação final com
  escopo por dono tem de ter sucesso depois de o corpo terminar antes de o
  desfecho ser reportado como cercado; um erro nessa última conferência
  responde `FreshUnfenced`, porque a posse é então desconhecida.

- **O chaveiro de criptografia é validado e instalado antes do bootstrap da
  aplicação.** O `Crypt` era inicializado pelo `Server::from_config`, então
  tudo o que rodasse mais cedo - o callback de bootstrap, um comando de
  console, um ponto de entrada de worker de fila que nunca constrói um
  servidor - ou não encontrava chaveiro nenhum ou construía o próprio. O
  `#[suprnova::main]` agora carrega o ambiente e depois valida e instala o
  chaveiro, nessa ordem, antes de o seu bootstrap rodar. A validação roda a
  cada boot mesmo depois de o chaveiro estar instalado, então um processo de
  produção com um `APP_KEY` ausente ou malformado ainda falha fechado,
  enquanto a chave de todo o processo continua imutável. O
  `APP_PREVIOUS_KEYS` do Laravel é aceito como alias para o
  `APP_KEY_PREVIOUS`; quando os dois estão definidos e discordam, o nome
  Suprnova vence e o duplicado é nomeado em um aviso.

- **O crate arquivado `proc-macro-error2` é substituído pelo seu sucessor
  mantido.** Ele foi arquivado em 2026-06-07, o RUSTSEC-2026-0173 o marca
  como sem manutenção, e ele fazia cada build avisar que uma versão futura
  do Rust vai rejeitá-lo (E0365). O `validator_derive` 0.20.1 passa para o
  `proc-macro-error3` 3.1.1 e o `sea-bae` 0.2.2 abandona a dependência,
  então nem o aviso de segurança nem o alerta aparecem mais. Isso chegou
  à main depois da tag `v2.0.0`; o `Cargo.lock` da tag ainda resolve o
  `proc-macro-error2`.

### Adicionado

- **O Suprnova Live faz parte do framework.** O `suprnova::live` é um motor
  de interação dirigido pelo servidor: um componente é um struct Rust cujo
  estado vive no servidor, cuja view é um template Askama verificado e cujas
  ações rodam sobre um protocolo assinado a partir de um pequeno runtime de
  navegador que aplica morph no HTML re-renderizado no lugar. Não há modelo
  de estado no lado do cliente para manter sincronizado, nem ferramenta de
  build para instalar para usar o runtime distribuído, nem JavaScript inline
  nos seus documentos. O motor é distribuído como um crate interno de que o
  framework depende incondicionalmente, então não há nada a habilitar; a
  metade de navegador é publicada como `@suprnova/live` e os bytes exatos
  revisados dela são servidos pelo próprio framework. O `manual/live.md` é o
  capítulo voltado à aplicação, e um projeto criado pelo `suprnova new` já
  vem pronto para o Live: ele escreve `src/live/mod.rs` com um registro
  vazio e uma função `routes()`, vincula o registro no `bootstrap.rs` e
  instala as rotas a partir do `cmd/main.rs`.

- **Componentes são declarados com `#[derive(LiveComponent)]` e `#[live]`.**
  O derive nomeia o componente e a view dele (`#[live(name = "app.counter",
  view = "live/counter.html")]`); o atributo `#[live]` no bloco `impl` marca
  os métodos que o navegador pode invocar. Um campo `#[public]` é
  renderizado e carregado no snapshot assinado, um campo `#[model]` além
  disso aceita propostas do navegador através do `live:model`, e um método
  `#[action]` é o único ponto de entrada que uma solicitação consegue
  alcançar, recebendo argumentos validados e retornando desfechos tipados
  como um redirecionamento ou um flash. Todo tipo de campo tem de
  implementar `Default`; uma ilha nova parte desses padrões, a menos que um
  hook de mount diga outra coisa. Componentes são registrados explicitamente
  através do `LiveRegistry::builder`, e o registro é imutável assim que o
  runtime se monta - um nome ou uma view duplicados, ou um componente cujas
  ações precisam de validação sem nenhuma porta de validação vinculada,
  falham no registro com um `RegistryError` tipado.

- **O `suprnova::view` é um contrato de view renderizada no servidor e
  verificada, para rotas comuns tanto quanto para componentes Live.** O
  `#[suprnova::view(path = "...")]` declara um template, o `TrustedHtml` é o
  único tipo auditado que um template pode emitir sem escape, e o filtro
  `trusted_html` é como ele chega lá; o `#[suprnova::view_filter]` declara
  um filtro customizado verificado. O Askama é o substrato, mas os handlers
  dependem dos contratos do próprio framework, e não dos módulos do motor de
  templates, e o `TemplateFailure` é um conjunto de falhas fechado e
  redigido (`MissingData`, `InvalidData`, `Failed`), e não o texto de erro
  do próprio motor.

- **A gramática de diretivas `live:` é fechada e provada contra os seus
  componentes.** Uma view vincula comportamento com `live:click`,
  `live:submit`, `live:model`, `live:upload`, `live:key`, `live:loading` e o
  resto do conjunto documentado - nunca uma linguagem de expressões inline,
  e nunca um script devolvido pelo servidor. O `suprnova live:check`
  constrói a sua aplicação e roda o verificador integrado sobre toda view
  registrada: uma ação desconhecida, um campo de model desconhecido, um
  filtro `safe` cru ou uma violação de acessibilidade falham com o arquivo,
  a linha e a coluna. O `--allow-unproved` aceita as estruturas dinâmicas
  sobre as quais o verificador deliberadamente não faz afirmação nenhuma.

- **O `Router::try_live()` instala o namespace Live reservado uma única
  vez.** Ele registra `/__live/v1/action`, `/__live/v1/upload`, as rotas de
  controle `/__live/v1/async/*` e o handshake de WebSocket, e as rotas
  imutáveis `/__live/v1/assets/*`, e a inicialização falha se uma rota da
  aplicação puder reivindicar `/__live`. O `Router::try_live_with` recebe um
  `LiveRouteGuard` cuja cadeia de middlewares é aplicada às rotas de ação,
  de upload e de controle assíncrono e ao upgrade de WebSocket, que é como
  uma aplicação vincula a própria autenticação, o próprio tenant e a própria
  limitação de taxa; rotas de asset ficam sem guard. Toda solicitação
  reservada carrega uma política estrita: fatos de sessão, de origem, de
  CSRF, de principal, de tenant e de limite de taxa têm todos de ter sido
  registrados por middleware real, e uma rota assíncrona que não consegue
  ver o conjunto completo é recusada em vez de abrir um transporte anônimo.

- **Uma solicitação Live prova a própria origem, e usar o Live não relaxa
  mais nada.** O runtime distribuído envia o tipo de mídia do Live e o
  próprio header `Sec-Fetch-Site` do navegador e não carrega token de
  sessão, então o `CsrfMiddleware` verifica essa prova sozinho para uma
  operação Live, qualquer que seja a política de origem que a aplicação
  configurou, e recai para a validação de token em uma solicitação
  cross-site ou sem header. Rotas comuns mantêm a política configurada,
  então uma aplicação não precisa mais alargar o `OriginPolicy` para a
  aplicação inteira só para o Live funcionar. O `AuthMiddleware::optional()`
  é a nova forma de guard de que isso precisa: ela registra um principal
  quando existe um e deixa uma solicitação anônima seguir, então visitantes
  anônimos podem agir sobre um seed público enquanto uma ilha vinculada a
  identidade ainda recusa uma solicitação sem evidência de principal.

- **Documentos posicionam ilhas através do `LiveDocument`.** Uma rota de
  documento constrói um a partir da solicitação, monta cada ilha com o
  `LiveMount` e emite a marcação de bootstrap exatamente uma vez. O
  `LiveMount::public_seed` declara uma ilha que qualquer visitante pode
  renderizar, cujo estado é um seed reutilizável promovido a uma instância
  real na primeira ação do visitante; o `LiveMount::identity_bound` declara
  uma ilha que pertence à sessão e ao principal atuais, então a rota de
  documento dela tem de autenticar. O `LiveDocument::bootstrap` emite o
  elemento de configuração inerte e as tags de script ordenadas com
  atributos de integridade para a estratégia ESM ou para a clássica,
  acrescenta os papéis de upload e de assíncrono quando um componente
  montado precisa deles e a ponte do Stimulus sob demanda, e recusa um
  segundo bootstrap ou um mount depois do bootstrap. O
  `Router::try_live_mount` registra um mount, e o
  `Router::try_live_document` declara uma rota de documento sem mounts de
  inicialização.

- **O framework serve os artefatos de navegador exatos e revisados.** As dez
  saídas de build determinísticas são embutidas e validadas contra o
  manifesto delas no primeiro uso, falhando fechado em qualquer desvio de
  digest, de comprimento, de nome de arquivo, de papel, de capacidade ou de
  versão, e são servidas a partir de `/__live/v1/assets/<identity>/<file>`
  para `GET` e `HEAD` com cache imutável, validadores fortes de digest,
  solicitações condicionais, `nosniff` e faltas fechadas. Documentos não
  contêm código executável inline, então uma política estrita `script-src
  'self'` se sustenta. O `suprnova live:assets --out <dir>` publica os
  mesmos bytes em um CDN ou em um diretório estático de forma atômica, trata
  uma publicação idêntica como já em dia, e recusa substituir um diretório
  cujos bytes diferem, a menos que você passe `--replace`.

- **Componentes Live aceitam uploads de arquivo sob uma política declarada e
  verificada.** Um atributo `#[upload(policy = ...)]` sobre um campo
  `#[model]` declara a contagem máxima de arquivos, os orçamentos de bytes
  declarado e agregado, os tipos de mídia aceitos e o comportamento de
  substituição através do `UploadPolicy::builder`, e a view o vincula com
  `<input type="file" live:upload="avatar">`. O runtime cria, transfere e
  completa o upload através do `/__live/v1/upload`; os bytes esperam em
  quarentena até a ação de finalização declarada rodar, quando o framework
  os entrega ao `UploadFinalizer` da aplicação, ao lado de um
  `UploadScanner` e de um `UploadApplicationValidator` opcionais. Todo
  controle é autorizado através do gate como
  `live:<component>.upload.<field>.<Control>` para cada um de `Create`,
  `Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
  `Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`
  e `Fail`. Toda solicitação revalida o mount, o principal, a sessão, o
  tenant, o componente, o campo e o escopo de documento atuais, um lock por
  handle serializa as corridas de chunk, de conclusão, de cancelamento, de
  ação, de finalização e de limpeza, e corpos de chunk reservam o orçamento
  compartilhado em voo antes de bufferizar. O
  `Router::try_live_upload_reacquisition` declara um caminho de propriedade
  da aplicação fora do namespace reservado onde um navegador que perdeu a
  concessão de transferência dele consegue uma nova, respondendo apenas à
  sessão e ao principal que criaram o upload.

- **Ilhas se atualizam de forma assíncrona por SSE, WebSocket ou polling.**
  Um componente declara os streams que escuta no atributo `#[live]`
  (`streams(stream(name = "activity", topics("activity"),
  events(ActivityPosted)))`), o framework assina um descritor de assinatura
  limitado para o visitante, e o runtime de navegador abre um transporte
  nativo e recai para polling quando não consegue. Assinar é autorizado
  através da habilidade de gate `live:<component>.stream.<name>`; a
  aplicação publica através do `suprnova::live::LiveStreams`, com o
  `refresh` dizendo às ilhas assinadas para renderizarem do zero e o
  `event::<T>` entregando um payload tipado aos handlers registrados da
  ilha. O fan-out, a contagem de saltos e a entrega por documento são todos
  limitados. O polling é uma renderização do zero comum, então o estado se
  põe em dia, mas payloads de evento publicados enquanto um transporte
  estava indisponível não são reproduzidos, o que o runtime reporta como um
  stream degradado, e não como um stream atual.

- **O runtime de navegador é um pacote TypeScript estrito que é distribuído
  em um bundle core e em bundles opcionais.** O `@suprnova/live` faz
  bootstrap uma vez por documento, descobre ilhas, faz o parse da gramática
  fechada de diretivas, dá a cada ilha trabalho limitado e estado pendente e
  falho verdadeiro, aplica uma resposta somente depois de um morph
  bem-sucedido através de um adaptador privado do Idiomorph fixado, e
  preserva foco, estado de formulário, controllers, scroll e histórico
  através de morphs e navegações nativas. Os bundles opcionais `uploads`,
  `async` e `stimulus` se acoplam através de uma porta de feature tipada,
  nas formas ESM e clássica; o Stimulus nunca é embutido no core. Não há
  `eval`, nem `new Function`, nem script devolvido pelo servidor, nem
  linguagem de expressões inline em lugar nenhum dele.

- **Quatro comandos de CLI cobrem o fluxo de trabalho do Live.** O `suprnova
  live:make <name>` faz o scaffold de um componente em `src/live/`, da view
  dele em `templates/live/` e do registro dele no builder `registry()`,
  declara o módulo, valida todo alvo e recusa traversal e symlinks antes de
  escrever, escreve atomicamente, nunca sobrescreve, desfaz todo arquivo que
  uma execução que falhou tenha escrito, e consegue reportar uma execução de
  teste. O `suprnova live:check`, o `suprnova live:inspect` e o `suprnova
  live:assets` são clientes finos de um comando de console oculto do
  framework e de um protocolo de linhas JSON limitado e versionado, então a
  CLI não mantém dependência nenhuma do framework nem do motor e falha
  fechada, sem escritas, diante de qualquer coisa não suportada, obsoleta,
  truncada, grande demais ou inesperada. O `live:inspect` reporta o registro
  vinculado, os limites de configuração, as capacidades de upload
  instaladas, os serviços de runtime montados e a identidade de asset como
  booleanos de presença e contagens, nunca como estado ou segredos.

- **O `suprnova::live::testing` prepara o runtime e o catálogo de mounts de
  um router para testes em processo.** O `prepare_live_router_for_test` dá a
  um teste o mesmo runtime que o servidor monta, então um teste consegue
  decodificar o snapshot de uma ilha a partir do atributo
  `data-suprnova-live-snapshot` dela, enviar uma ação com um cookie de
  sessão real e `Sec-Fetch-Site: same-origin`, e fazer asserções sobre a
  renderização aceita através da pilha de middlewares globais real da
  aplicação.

- **O RenderCache armazena uma cópia provadamente segura da resposta de uma
  rota e serve a próxima solicitação correspondente sem rodar o handler.** É
  opt-in por rota e por grupo, nunca muda o que uma aplicação pode fazer, e
  uma rota que ele recusa ainda assim renderiza e é servida corretamente. O
  `Router::try_render_cache` opta por incluir um padrão de rota já
  registrado e o `Router::try_render_cache_group` opta por incluir toda rota
  sob um prefixo de caminho; o `RenderCache::install(router,
  RenderCacheConfig::from_env())` termina a ligação depois de todo registro
  de middleware que estabelece locale, sessão ou identidade com escopo de
  solicitação. O `RENDER_CACHE_ENABLED=false` é um botão de desligar de
  verdade no momento da instalação: uma configuração desabilitada devolve o
  router intocado, não sonda nada, não registra nada e deixa o gate do
  processo fechado.

- **Uma política de cache declara uma classe de representação, uma política
  de frescor e como a resposta pode ser compartilhada.** O
  `RenderCachePolicy::builder` recebe uma `RepresentationClass` que vai da
  mais ampla à mais estreita - `PublicShared`, `PublicShellStitched`,
  `PrivateCached`, `Uncacheable` - e a `FreshnessPolicy::new(fresh_ms,
  stale_servable_ms, stale_on_error_ms)` define por quanto tempo uma
  representação é fresca e depois até que ponto além dessa borda uma cópia
  armazenada pode ser servida enquanto uma reconstrução em segundo plano
  roda ou depois de uma reconstrução em primeiro plano ter falhado. A
  `SharedCachePolicy` controla o que se diz a um cache compartilhado à
  frente da aplicação. Uma rota dentro de um grupo em cache pode estreitar a
  política que a envolve com um `PolicyPatch` em vez de reafirmá-la, e só
  pode torná-la mais estreita; tirar uma rota de um grupo em cache é um
  patch que define a classe como `Uncacheable`.

- **A variância é declarada, nunca adivinhada.** Uma representação em cache
  varia por padrão de rota, por parâmetros de caminho e por build da
  aplicação, a menos que uma política diga outra coisa. O
  `QueryPolicy::declared([...])` nomeia os parâmetros de query string que
  distinguem representações, e qualquer outro parâmetro de query em uma
  solicitação faz aquela solicitação desviar do cache, em vez de ser
  ignorado em silêncio. O `.vary(VarianceDimension::Locale | ::Host |
  ::Tenant | ::Principal)` particiona pelo locale negociado, pelo host da
  solicitação, pelo tenant atual ou pelo visitante autenticado, os dois
  últimos como material de chave opaco; uma rota `PrivateCached` que não
  declara nem `Principal` nem `Tenant` nem chega a ser construída. `Media` e
  `Encoding` são declarados juntos com o conjunto fechado próprio deles
  através do `.vary_media(NegotiatedPolicy::declared([...], default)?)` e do
  `.vary_encoding(...)`: o middleware negocia o `Accept` ou o
  `Accept-Encoding` da solicitação contra esse conjunto conforme a RFC 9110,
  com a qualidade mais alta vencendo, a qualidade igual mantendo a ordem da
  esquerda para a direita do próprio header, um curinga comparado como um
  token literal em vez de expandido, e uma qualidade `q=0`, fora do
  intervalo ou não parseável excluindo um candidato em vez de dar um padrão
  a ele. Um header ausente, sem correspondência ou não parseável resolve
  para o padrão declarado e nunca entra em panic. Tanto a chave de
  renderização quanto o header `Vary` tiram o valor deles dessa mesma
  resolução única, então eles não conseguem discordar.

- **Um acerto servido é uma resposta HTTP de verdade com validadores de
  verdade.** Ele carrega o `ETag` como um validador forte que um cliente
  pode devolver como `If-None-Match` para um `304`, além de `Cache-Control`,
  `Vary` e `Age` em segundos inteiros desde a publicação, que é o sinal
  local mais rápido de que uma resposta saiu do armazenamento e não de um
  handler. Uma resposta servida além do intervalo de frescor dela carrega
  além disso `Warning: 110 - "Response is Stale"`. Solicitações condicionais
  e `HEAD` são respondidas a partir da entrada armazenada.

- **A saída em cache é provada atual contra o banco de dados, não
  presumida.** Um coletor com escopo de solicitação atribui toda leitura que
  um handler faz à coisa que ele leu: um model, uma tabela, um valor de
  configuração, uma feature flag, uma decisão de autorização, um eixo de
  identidade. Todo caminho de escrita suportado do outro lado avança a
  geração daquilo que mudou, na própria transação do chamador quando há uma -
  as escritas de model e em massa do ORM, a facade do query builder,
  escritas cruas de tabela, o caminho de hidratação de pagamentos, escritas
  de feature flag e os statements de papel e de permissão do RBAC, incluindo
  as novas revogações. Um acerto reprova as gerações de que depende antes de
  servir, seja relendo o ledger sob o `CoherenceMode::Authority`, seja
  contra um lease de validação sob o `CoherenceMode::Lease`, então nada que
  uma escrita tenha invalidado pode ser servido como atual. O
  `RenderCache::bump_permission_version` é a única invalidação que uma
  aplicação chama à mão, a partir do caminho de código que muda o que um
  usuário autenticado pode fazer; ela avança uma geração persistida que toda
  renderização chaveada por principal observa, sobrevive a um restart e
  entra na transação em que a mudança de papel roda.

- **Autorização, feature flags e escopos globais participam honestamente.**
  Uma decisão de gate é julgada pelo eixo de identidade que a avaliação dela
  de fato consultou, então uma consulta só de tenant precisa apenas de
  `Tenant` declarado, enquanto qualquer coisa que tenha resolvido material
  de principal, ou não tenha resolvido nada nomeável, precisa de
  `Principal`. Uma rota protegida por RBAC entra em cache e uma concessão ou
  revogação de permissão a reconstrói, porque as cinco tabelas de papel e de
  permissão são observadas em vez de tratadas como uma incógnita. Uma
  leitura de feature flag observa uma geração `Feature` sempre que o
  snapshot mantém aquela flag em qualquer chave de escopo, o `set_flag` a
  avança, e uma recarga de flags a avança para toda flag que o diff dela
  própria encontrou alterada. Um `GlobalScope` do Eloquent declara
  `ScopeDependency::Constant` ou mantém o padrão conservador `PerRequest`, e
  um escopo por solicitação cujo filtro leu algo que o coletor não consegue
  nomear é registrado como uma leitura não declarada e estreita a
  renderização para `Uncacheable`, em vez de colocar um filtro de tenant em
  cache em silêncio. O `suprnova::live::current_tenant()` é o acessor
  instrumentado que um corpo de gate ou um escopo procura.

- **O lado de escrita está aberto em todo processo que escreve através do
  ORM.** Um worker de fila, uma tarefa agendada ou um comando de console
  escreve através do mesmo ORM que o servidor usa e nunca chama o
  `RenderCache::install`, então as escritas dele não avançavam geração
  nenhuma e páginas que dependiam delas seguiam sendo servidas obsoletas. O
  lado de escrita agora é um tri-estado de todo o processo sondado no máximo
  uma vez, nunca dentro da transação de um chamador, então todo processo que
  escreve avança as mesmas gerações que o servidor avança, enquanto uma
  aplicação com o cache desabilitado continua não emitindo SQL nenhum do
  RenderCache.

- **Três perfis de deployment decidem onde vivem as entradas e a liderança
  de reconstrução.** O `RENDER_CACHE_PROFILE` seleciona `embedded` (uma
  camada de arquivos por processo sob o `RENDER_CACHE_L1_DIR`, liderança em
  processo), `database` (entradas em `suprnova_render_entries`, leases em
  `suprnova_render_leases`, registros de instância Live em
  `suprnova_live_instances` e `suprnova_live_promotions`) ou `redis` (um
  hash do Redis por chave, mais um contador de token de publicação por
  chave). O `RENDER_CACHE_L1` e o `RENDER_CACHE_COORDINATOR` sobrescrevem
  qualquer uma das duas metades de forma independente, então um deployment
  que quer as entradas dele no banco de dados e os leases dele em processo
  diz exatamente isso. A verdade sobre gerações não se move: o ledger
  apoiado em banco de dados é a autoridade em todo perfil, que é o que deixa
  o Redis perder tudo o que guarda sem que nada obsoleto seja provado atual.
  A tabela completa é `RENDER_CACHE_ENABLED`, `RENDER_CACHE_PROFILE`,
  `RENDER_CACHE_L1`, `RENDER_CACHE_COORDINATOR`, `RENDER_CACHE_L0_ENTRIES`,
  `RENDER_CACHE_L0_BYTES`, `RENDER_CACHE_L1_DIR`, `RENDER_CACHE_L1_BYTES`,
  `RENDER_CACHE_REDIS_URL`, `RENDER_CACHE_REDIS_PREFIX`,
  `RENDER_CACHE_LEASE_MS`, `RENDER_CACHE_MAX_WAITERS`, `RENDER_CACHE_HINTS`,
  `RENDER_CACHE_FAILURE` e `APP_BUILD_ID`. Uma variável de conjunto fechado
  que receba um valor fora do conjunto dela faz o boot falhar com uma
  mensagem que nomeia a variável e nunca repete o valor, porque um valor de
  ambiente pode carregar um segredo. O ledger de instâncias Live tem os
  próprios `LIVE_LEDGER_DRIVER`, `LIVE_REDIS_URL` e `LIVE_REDIS_PREFIX`, e
  as duas instalações falham fechadas no boot diante de uma migração de
  camada ausente ou de um endpoint que ninguém responde.

- **Um documento Live pode ser colocado em cache como um shell compartilhado
  com ilhas por visitante.** Uma rota que declara
  `RepresentationClass::PublicShellStitched` armazena o shell uma vez como
  uma entrada composta recortada a partir dos slots que o
  `LiveDocument::mount` capturou, e em um acerto o middleware anexa a
  entrada preparada e ainda assim chama a cadeia da rota, então o guard e o
  middleware de tenant da própria rota decidem a solicitação antes de o
  middleware de conclusão do Live remontar cada slot sob autoridade derivada
  só para aquela solicitação. Uma captura que não seja exatamente utilizável -
  um slot não encontrado exatamente uma vez, um digest de documento que
  não bate - recusa a publicação e não armazena nada. Os limites são 32
  slots, 64 buracos de nonce, 193 segmentos de grafo e 4.096 bytes cada para
  parâmetros de slot e fallbacks. Como toda montagem é uma representação
  distinta, resposta composta nenhuma responde `304` nem honra o
  `If-None-Match`, e a uma montagem com slots se envia `Cache-Control:
  private, no-store`, enquanto uma sem slot nenhum mantém o `max-age`
  privado da classe dela.

- **Um composto armazenado pode nomear outra entrada armazenada como um dos
  segmentos dele.** O `LiveNestedSegment` é a declaração tipada e o
  `Router::try_live_nested_segment` o registra. A entrada interna mantém a
  chave própria e a versão própria, então ela é invalidada, republicada e
  cercada nos termos dela e não nos de quem a inclui, e cada segmento
  declara o que acontece quando ele não pode ser resolvido: falhar o
  documento, omiti-lo ou servir um fallback limitado. O aninhamento é
  limitado a três níveis e dezesseis segmentos aninhados, um ciclo é
  reportado como um ciclo mesmo quando ele também estouraria a profundidade,
  e o comprimento montado é conferido contra o limite de corpo antes de um
  único byte ser copiado. A publicação é recusada para um composto que
  nomeie um segmento interno de uma classe de representação mais ampla, um
  com uma janela de frescor mais longa, um ciclo transitivo, um grafo além
  do limite de profundidade ou um segmento interno `PrivateCached` que nunca
  poderia ser resolvido. Em um acerto, um segmento interno vinculado a
  identidade é reautorizado para o visitante que solicita; um declarado
  livre de identidade pula isso, o que é liberdade de identidade provada, e
  não um enfraquecimento dela.

- **Nós conseguem dizer uns aos outros que uma geração acabou de se mover.**
  O `RENDER_CACHE_HINTS` liga um canal de pub/sub do Redis que carrega os
  digests de dependência que um avanço acabou de tocar, ligado por padrão
  para o perfil `redis` e desligado para os outros dois, pegando carona no
  mesmo `RENDER_CACHE_REDIS_URL` e `RENDER_CACHE_REDIS_PREFIX` do próprio
  cache. O único poder de uma dica é fazer um nó revalidar mais cedo do que
  o lease dele teria feito: ela nunca consegue estender ou criar um lease,
  provar uma entrada atual, desviar da leitura do ledger que um acerto ainda
  faz, nem tocar na época de autoridade, que é por que o canal é não
  autenticado por design e por que um endpoint de dicas inalcançável não
  recusa o boot do jeito que uma camada de cache inalcançável recusa. Uma
  dica não carrega instante nenhum, então nenhuma suposição de desvio de
  relógio entre nós é necessária. Um deployment com as dicas desligadas, um
  cujo canal está morto e um que nunca as teve servem as mesmas entradas e
  admitem as mesmas reconstruções; só o momento da revalidação difere.

- **Dois comandos de console e nove contadores de telemetria são a
  superfície de operação.** O `render-cache:inspect <key>` reporta a classe
  de representação de uma entrada armazenada, o `body_bytes`, outros
  metadados e a época de autoridade atual, e lê a camada em processo deste
  processo e mais nada, então em um perfil compartilhado ele responde "o que
  este nó tem em memória" em vez de "o que o deployment tem armazenado". O
  `render-cache:epoch-advance` é a invalidação de emergência: ele avança a
  época de autoridade, que está assada em toda chave de busca, então
  entradas armazenadas ficam fora de alcance sem nada a enumerar e nada a
  excluir, e no nó que o roda o efeito é imediato. Nenhum dos dois jamais
  imprime um corpo armazenado ou uma identidade de dependência crua, o que é
  afirmado em teste em vez de meramente declarado. Os contadores são
  `suprnova.render_cache.lookups`, `.hits`, `.publications`, `.rebuilds`,
  `.stitch.assemblies`, `.stitch.slots`, `.stitch.nested`, `.hints` e
  `.epoch_rewinds`, todos com atributos fechados de baixa cardinalidade que
  nunca nomeiam uma rota, uma chave, um digest, uma camada ou um provider.

- **Uma busca recusada diz exatamente qual contrato a recusou.** O
  `outcome="declined"` no contador de buscas agora carrega um atributo
  `reason` vindo de um conjunto fechado de trinta e oito rótulos, calculado
  a partir de um valor tipado no ramo que de fato recusou, em vez de
  reconstruído depois a partir da resposta. Eles são agrupados por contrato:
  elegibilidade (`policy_uncacheable`, `method`, `status`, `streaming`,
  `sets_cookie`, `unsafe_header_name`), observação
  (`observation_overflowed`, `ledger_read_failed`, `handler_not_begun`),
  classificação, divergência de chave, documento Live e construção de
  composto. Todo rótulo está documentado no capítulo de operações, e um
  teste afirma isso em vez de confiar na prosa. O `reason` é emitido somente
  ao lado do `outcome="declined"`; um acerto e uma falta não carregam
  nenhum.

- **Uma época de autoridade que anda para trás é detectada, recusada e
  ultrapassada.** Uma restauração de banco de dados pode mover a época para
  um valor que o deployment já usou, o que deixaria entradas publicadas sob
  o valor antigo, mais alto, serem provadas atuais de novo. Uma entrada ou
  um lease carimbados acima da autoridade agora são recusados em qualquer
  idade, antes de qualquer comparação de dependência, e o nó que detecta
  isso eleva a época do ledger para além do carimbo, larga o lease dele,
  limpa a camada em processo dele e incrementa o
  `suprnova.render_cache.epoch_rewinds`.

- **O NOWPayments se junta ao Stripe e ao Paddle como adaptador de
  pagamento.** O crate `suprnova-payments-nowpayments` cria faturas
  hospedadas, verifica notificações de pagamento e lê o status de pagamento
  com a chave de API do lojista, e se registra como `nowpayments` no
  registro comum de providers através do `NowPaymentsProvider::from_env()`.
  Ele lê o `NOWPAYMENTS_ENVIRONMENT` (`sandbox` ou `production`, com
  `sandbox` como padrão; um valor desconhecido ou em branco falha a
  configuração), o `NOWPAYMENTS_API_KEY`, o `NOWPAYMENTS_IPN_SECRET` e o
  `NOWPAYMENTS_IPN_CALLBACK_URL`, e recusa credenciais em branco antes de
  qualquer solicitação HTTP. O endpoint de webhook dele é o `POST
  /webhooks/payments/nowpayments` através das `webhook_routes`
  compartilhadas, e ele precisa da URL de callback HTTPS pública exata e de
  corpos de solicitação que cheguem ao adaptador sem alteração. O
  `manual/payments-nowpayments.md` é o capítulo.

- **O RBAC tem a contraparte que revoga de todo helper que concede.** A
  superfície exportava as metades que concedem e que checam de uma API de
  controle de acesso e nada que tirasse acesso, então remover o papel de um
  administrador significava compor statements contra tabelas de junção cuja
  semântica você tinha de inferir - durante um incidente, que é quando se
  recorre à revogação. O `remove_permission_from_role`, o
  `remove_role_from_model` e o `remove_permission_from_model` são as funções
  livres, cada uma com o par `_on_guard` que o lado que concede já tinha, e
  o `HasRoles::remove_role` e o `HasRoles::remove_permission_to` fazem par
  no ponto de chamada. Um nome que não existe em guard nenhum desses é um
  erro, então um erro de digitação ou uma chamada mirada no guard errado é
  barulhenta; uma atribuição que o model ou o papel não tem é um no-op que
  retorna `Ok`, então repetir uma revogação que já aterrissou é seguro. Cada
  chamada remove exatamente a única atribuição que ela nomeia, sem varredura
  em massa, e todo statement resolve o executor dele através da transação
  ambiente primeiro, então um pacote de conceder e revogar dentro de um
  `DB::transaction` faz commit ou rollback como uma unidade só. A revogação
  é específica da fonte e não contradiz o `has_permission_for_model`, que
  resolve uma concessão direta antes de uma herdada de papel: tirar um papel
  deixa uma permissão que o model também tem diretamente ainda respondendo
  verdadeiro, e o rustdoc nomeia a segunda chamada que encerra o acesso
  efetivo.

- **Uma aplicação consegue construir o router dela de forma assíncrona.** O
  `Application::try_routes_async` recebe uma closure que retorna um future,
  e o `Server::try_from_config_with_routes_async` é o gêmeo assíncrono do
  `try_from_config_with_routes` que a hospeda, compartilhando o mesmo
  prólogo e o mesmo epílogo. Ele existe porque o `RenderCache::install` tem
  de sondar as tabelas do ledger de gerações antes de conseguir montar um
  runtime, e a closure de rotas é o único lugar que tem ao mesmo tempo um
  container e um router - hook de boot nenhum tem os dois. O `routes`, o
  `try_routes` e o `try_routes_async` escrevem o mesmo slot, então o último
  chamado é o que o servidor constrói.

- **A integração com o Magnetar expõe o que um sign-in parcialmente
  concluído de fato retornou.** O `SignInOutcome` é público, então um
  callback de magic link, de OAuth ou de passkey que resolve para
  `SignInOutcome::FactorRequired` pode ser tratado em vez de ser reportado
  como uma falha: a sessão do framework não é vinculada, e o seletor que ele
  carrega pode ser completado através do host engine retido. O `FactorAuth`
  e o `MagnetarFactorAuthEngine` são os tipos que seguram essa continuação,
  e o `install_magnetar_oauth_engine` e o
  `install_magnetar_oauth_engine_with_factor` instalam um engine de OAuth
  com ou sem um. Falhas de bootstrap do Magnetar agora nomeiam o que está
  faltando em vez de reportar um erro de instalação genérico.

- **O contrato de driver de fila reporta se ele consegue honrar um filtro
  por nome de fila.** O `QueueFilterCapability` (`Supported`, `Unsupported`,
  `Unknown`) é o que o `QueueDriver::queue_filter_capability` retorna; o
  padrão é `Unknown`, e não `Unsupported`, então um driver de terceiros que
  já sobrescreve o `pop_from` continua funcionando inalterado, e um
  decorador pode recusar uma conexão sabidamente `Unsupported` antes de
  fazer poll, mas tem de deixar um driver `Unknown` responder por si. O
  `TerminalCallbackClaim` são os metadados que um worker recebe de volta
  quando reivindica atomicamente os callbacks terminais de um batch
  terminado, carregando o horário de conclusão durável e qualquer
  cancelamento visível na mesma seção crítica, para que a decisão de
  callback não possa ser tomada a partir de um snapshot que ficou obsoleto
  antes de a posse ser eleita.

- **O `Schedule::try_add` é o irmão falível do `Schedule::add`.** O nome da
  tarefa identifica uma tarefa na busca direta e nas chaves distribuídas
  usadas pelo `TaskBuilder::on_one_server` e pelo
  `TaskBuilder::without_overlapping`, então um nome não pode identificar
  duas entradas registradas, e um agendamento que registrasse o mesmo nome
  duas vezes tinha duas tarefas disputando um lock. Os nomes agora são
  exatos, sensíveis a maiúsculas e minúsculas, e únicos: o `add` entra em
  panic diante de uma duplicata e a tarefa existente é mantida, enquanto o
  `try_add` retorna o erro para o código que preferir tratá-lo.

- **Os limites de tempo de vida de sessão e o erro de migração de sessão são
  públicos.** O `MAX_SESSION_LIFETIME_SECS` e o
  `MAX_SESSION_LIFETIME_MINUTES` são os limitadores que o parsing de
  ambiente e o driver de banco de dados aplicam, e o `SessionMigrationError`
  é exportado para que um `SessionStore` customizado consiga nomear a falha
  que ele retorna.

- **Projetos com scaffold são construídos em formato de produção por
  construção.** O `Cargo.toml` gerado depende do framework com as features
  padrão desligadas e os nove padrões que não são `testing` listados
  explicitamente (`filesystem`, `database-sqlite`, `database-postgres`,
  `database-mysql`, `vector-mariadb`, `web-push`, `localization`,
  `magnetar-oauth`, `media`), e reacrescenta `features = ["testing"]` como
  dependência de desenvolvimento. O resolvedor do Cargo puxa as features de
  uma dependência de desenvolvimento apenas para o `cargo test` e outras
  builds `--tests`, então o `cargo build --bin app` nunca compila uma
  costura de teste dentro de um binário distribuído, e o `cargo test` fica
  inalterado. O `manual/deployment.md` documenta o formato para uma
  aplicação existente adotar.

- **O id de build padrão do cache vem da aplicação, e não do framework.** O
  `#[suprnova::main]` registra o `CARGO_PKG_VERSION` do próprio crate da
  aplicação logo depois de carregar o ambiente, e o
  `RenderCacheConfig::from_env` resolve o `build_id` através de um
  `APP_BUILD_ID` explícito, depois daquela versão de aplicação registrada,
  depois da versão do próprio crate deste framework, somente para um binário
  que nunca expandiu o `#[suprnova::main]`. O
  `RenderCacheConfig::with_build_id` sobrescreve o que quer que o `from_env`
  tenha escolhido, para uma aplicação que deriva o próprio identificador por
  deploy em código. Defina o `APP_BUILD_ID` explicitamente uma vez por
  deploy: ele é misturado em toda chave de busca, e uma versão de pacote
  raramente muda quando você publica um template, uma tradução ou uma
  correção de handler.

### Alterado

- **Um worker na conexão de fila com failover agora drena toda conexão, e
  não só a primária.** O `FailoverQueueDriver` documentava a consequência
  que herdou do Laravel: as escritas caíam pela lista, as leituras não,
  então o que fizesse failover para um fallback ficava ali até alguém rodar
  um segundo worker contra aquele fallback diretamente. O `pop` e o
  `pop_from` agora rotacionam a conexão inicial deles e depois varrem a
  lista inteira sequencialmente - rotação para que uma primária recuperada e
  continuamente ocupada não consiga matar de fome o trabalho que aterrissou
  em um fallback, sequencial para que uma chamada não consiga reservar
  vários jobs e devolver um. Cada reserva recebe um token agregado novo que
  o driver mapeia de volta à conexão que realmente é dona dele, porque
  tokens internos não são globalmente únicos e dois backends podem
  legitimamente cunhar o mesmo UUID; um token agregado expirado ou
  desconhecido é tratado como obsoleto em vez de enviado a uma conexão
  arbitrária. Os contadores e as três listagens agregam toda conexão
  configurada na ordem configurada, e o `clear` tenta todas elas, então o
  que um operador inspeciona é o backlog que este driver consegue de fato
  consumir. Um driver declara se consegue honrar um filtro por nome de fila
  através do `QueueDriver::queue_filter_capability`, que tem `Unknown` como
  padrão, então um driver de terceiros existente não é afetado.

- **O lease mínimo de workflow é de dois segundos, e o primeiro heartbeat
  dispara imediatamente.** O heartbeat renova a `max(lock_timeout / 2, 1s)`,
  então um lease de um segundo estava previsto para a primeira renovação
  dele na própria expiração ou depois dela: qualquer latência de
  reivindicação ou jitter de agendamento abria uma janela por onde outro
  worker podia passar enquanto o primeiro já estava rodando efeitos. O
  primeiro tick do heartbeat não é mais pulado, o que fecha a janela entre a
  reivindicação e a primeira renovação, e a admissão aguarda uma renovação
  com posse antes de qualquer código de usuário rodar, e não apenas dentro
  de um passo. Um `WORKFLOW_LOCK_TIMEOUT_SECS` abaixo de dois é limitado com
  um aviso que diz o porquê, e uma configuração construída em código que
  carregue um lease mais curto falha na validação.

- **Um provider de pagamento sem registros de cliente pode recusar a
  hidratação do espelho de transações.** O
  `WebhookHandler::mirrors_payment_transactions` tem `true` como padrão,
  então todo provider existente se comporta como se comportava; um provider
  que retorna `false` continua tendo os eventos verificados dele persistidos
  e deduplicados no log de auditoria de webhooks, reembolsos incluídos, mas
  nenhum espelho de transação é fabricado para pedidos de que a aplicação é
  dona e que ela tem de reconciliar contra o estado autenticado do provider.
  O `try_extract_payment_snapshot` é a forma falível do
  `extract_payment_snapshot` que o caminho de hidratação usa: retornar `Err`
  deixa o webhook pendente para que o provider o repita, enquanto o
  `Ok(None)` fica reservado para um evento que genuinamente não consegue
  fornecer um snapshot completo. Os dois são métodos providos, então um
  driver existente compila e se comporta inalterado.

- **Um `transaction.billed` do Paddle não é mais lido como dinheiro
  recebido.** Uma fatura emitida não confirma recebimento, e tratá-la como
  uma liquidação marcava como pagos pedidos que não estavam. Ajustes de
  reembolso e de disputa aprovados são classificados como ajustes em vez de
  transações, com a moeda deles tirada de `data.currency_code` e o horário
  de liquidação deles tirado da última tentativa de pagamento capturada
  quando há uma disponível; só o `WebhookHandler::parse_event` os
  classifica, porque a decisão precisa da ação e do status de aprovação do
  próprio payload. O adaptador também preserva a correlação de lojista que
  recebeu através do checkout, em vez de substituí-la por um identificador
  de cliente que o Paddle.js não aceita como token de autenticação de
  cliente, codifica dados customizados não string como strings JSON de forma
  consistente entre as solicitações de cliente e de checkout, e dá ao
  cliente HTTP dele o prazo de solicitação que o SDK fixado não define.

- **O crate do framework carrega três módulos novos e duas dependências
  duras novas.** O `suprnova::live`, o `suprnova::render_cache` e o
  `suprnova::view` são incondicionais, e não protegidos por feature, então o
  framework agora depende do crate de motor interno `suprnova-live` e do
  `askama` em toda build. Nenhum dos dois módulos faz coisa alguma até uma
  aplicação optar por incluí-lo: o `Router::try_live()` é o que instala as
  rotas reservadas do Live, e o `RenderCache::install` com uma política é o
  que faz o cache fazer qualquer coisa.

### Corrigido

- **Um job de batch não é reconhecido até a contabilidade dele estar
  durável.** O worker reconhecia um membro de batch bem-sucedido e depois
  escrevia a escrituração do batch, então uma falha entre os dois deixava um
  batch permanentemente com uma liquidação a menos, e os callbacks de
  conclusão dele nunca disparavam. A reserva agora é mantida até toda
  escrita de contabilidade ter sucesso; uma escrita recusada ou incerta a
  deixa intacta, para que a expiração de visibilidade reentregue o job, e a
  unicidade de `(batch_id, job_id)` do repositório torna a repetição segura
  mesmo quando a primeira escrita fez efeito e só a resposta dela se perdeu.
  O repositório de batches é instalável separadamente e pode endereçar um
  banco de dados diferente, que é por que isto não pode simplesmente
  compartilhar a transação de liquidação da fila.

- **Os callbacks terminais de um batch são eleitos exatamente uma vez.**
  Dois jobs terminando as duas últimas entradas de um batch podiam observar
  os dois uma contagem de pendentes de zero e rodar os dois os callbacks de
  conclusão. As linhas de liquidação agora são a fonte da verdade e a linha
  do batch pai delas é travada antes da inserção, então jobs concorrentes de
  um batch formam uma ordem total e exatamente uma liquidação final observa
  zero - através de locks de linha no PostgreSQL e no MySQL, e através do
  lock de escritor que a transação serializada toma no SQLite. Reivindicar o
  pacote de callbacks devolve o horário de conclusão durável e qualquer
  cancelamento visível naquela mesma seção crítica, então um worker não
  consegue escolher o `then` a partir de um snapshot que ficou obsoleto
  antes de a posse ser eleita, e um batch não vazio cuja contagem de
  pendentes chegou a zero fica selado contra crescimento positivo. Um
  cancelamento já visível depois de uma resposta incerta não é recarimbado
  na reentrega. Falhas de construção de envelope coletadas enquanto se monta
  um batch pendente são trazidas à tona no dispatch, que recusa o batch
  inteiro antes de qualquer mutação de repositório ou de driver, em vez de
  serem engolidas por um builder fluente infalível.

- **O driver de fila do Redis cerca com fence toda operação terminal contra
  a entrega para a qual ela foi emitida.** O `ack`, o `nack`, o `release` e
  o `settle` agora comparam o dono consumidor e a contagem de entregas da
  entrada de stream com a geração capturada pelo `pop`, e um único script
  Redis aplica qualquer publicação sucessora e o `XACK` juntos, então uma
  resposta atrasada que faz o chamador repetir encontra a geração já ida, e
  a repetição vira um no-op em vez de uma publicação duplicada. Como o
  `nack` é inerentemente dois comandos (`XADD` e depois `XACK`), cada
  reserva retém um ciclo de vida por token que continua endereçável através
  de toda operação que falhe e só é removido depois de o reconhecimento ter
  sucesso, então uma repetição retoma no passo que falhou em vez de
  republicar. O contrato de pelo-menos-uma-vez do driver e a exigência de
  que os handlers sejam idempotentes agora estão declarados na documentação
  do módulo em vez de ficarem implícitos. Identidades de consumidor são
  isoladas por processo, então dois workers não conseguem reivindicar as
  entradas pendentes um do outro.

- **Uma reserva de fila de banco de dados dura o tempo que foi pedido.** O
  `reserved_until` armazena segundos inteiros e os leitores o comparam com o
  tempo atual arredondado para baixo, então o segundo fracionário atual era
  silenciosamente tirado de todo lease e um timeout de menos de um segundo
  podia arredondar para nada. O instante absoluto de expiração agora é
  arredondado para cima.

- **Escritas de passo de workflow são cercadas com fence, um orçamento de
  tentativas esgotado vira terminal, e colunas de data do MySQL batem com as
  entidades que as leem.** Uma escrita de passo vinda de um worker que já
  tinha perdido a reivindicação dele podia aterrissar por cima do worker que
  agora é dono do workflow; as escritas agora carregam o token de fencing da
  reivindicação. Uma linha cujo orçamento de tentativas já está esgotado não
  podia nem ser reivindicada nem ser deixada pendente para sempre: o
  statement de reivindicação agora torna terminal no máximo uma linha dessas
  por poll, com predicados de limpeza e de reivindicação disjuntos, para que
  um backlog abandonado grande não consiga transformar um poll de worker em
  uma escrita ilimitada. As migrações iniciais de workflow declaravam
  colunas de data do MySQL como `TIMESTAMP`, enquanto as entidades públicas
  usam `chrono::NaiveDateTime`, cujo tipo de armazenamento no MySQL é
  `DATETIME`; a `NormalizeWorkflowDateTimesForMysql` é uma migração aditiva
  que as converte e é um no-op no PostgreSQL e no SQLite.

- **Cancelar uma task não abandona mais os efeitos diferidos de uma
  transação.** Uma `DB::transaction` abortada desfaz o trabalho de banco de
  dados dela quando o SeaORM larga a transação, mas nada desfazia um push de
  fila diferido nem liberava um lock de unicidade mantido para ela, e um
  callback pós-commit que já estava rodando quando o abort aterrissou era
  largado no meio do efeito. Os callbacks agora rodam como tasks filhas
  aguardadas, na ordem de registro, então um que já esteja em voo sobrevive
  ao cancelamento de quem o chamou, e o resto que não começou é desviado
  para uma task destacada que ou roda a lista de pós-commit (a transação de
  fato fez commit) ou compensa (ela não fez). Um callback que entra em panic
  surge como um erro em vez de pular os callbacks atrás dele, e um `COMMIT`
  que o banco de dados recusa depois de a closure já ter tomado a
  solicitação é tratado em vez de entrar em panic.

- **Cópias entre discos e o cache read-through limpam depois de uma task
  cancelada.** Uma falha no meio do stream já descartava o objeto de destino
  parcial, mas um cancelamento não retorna erro nenhum, então o writer era
  simplesmente largado e um objeto truncado ou um upload multipart preparado
  ficava para trás. O writer de destino agora pertence a um guard ao longo
  da transferência: um erro se resolve na hora com o mesmo abort e delete de
  antes, enquanto um cancelamento desvia essa limpeza para uma task
  destacada, e a própria limpeza roda até o fim mesmo que a task que a
  aguarda seja cancelada durante ela. A limpeza nunca mira um objeto
  publicado, porque outro writer pode ter vencido a condição. O trabalho de
  sistema de arquivos local é mantido vivo até terminar, já que largar um
  future de sistema de arquivos do Tokio não interrompe o trabalho
  bloqueante que ele já submeteu.

- **Duas cláusulas de throttle que faziam hash para a mesma identidade de
  armazenamento não compartilham mais um contador.** Uma regra de limitação
  de taxa com várias cláusulas finitas podia colidir, então os hits de uma
  cláusula contavam contra o orçamento de outra. Cláusulas que colidem agora
  reservam identidades de contador e de temporizador determinísticas e
  inequívocas, calculadas uma vez para que o gate, o hit diferido e os
  headers de resposta usem todos a mesma chave, e chaves antigas são
  mantidas onde não há colisão.

- **Uma varredura de janela deslizante não apaga mais o histórico de que uma
  cota mais longa ainda precisa.** O limitador em memória descartava um
  bucket cujo último hit fosse mais antigo do que a janela sobre a qual lhe
  perguntaram, o que jogava fora o registro que aplicava uma cota mais longa
  já observada na mesma chave. Um bucket agora é retido até o último hit
  registrado dele ser mais antigo do que a janela fornecida e do que a maior
  janela de cota observada para ele. Um decremento abaixo da quantidade
  mínima é tratado em vez de subcontado.

- **O driver de cache em memória recusa um incremento contra um valor não
  inteiro.** O `INCRBY` e o `DECRBY` do Redis recusam um valor não inteiro
  vivo e deixam o valor e o TTL dele em paz; o driver de memória o
  sobrescrevia, então o mesmo código se comportava de forma diferente contra
  os dois backends. Ele agora faz o parse antes de inserir e deixa a entrada
  intocada em caso de erro. Em separado, o `add` do Redis instala um valor
  sem tag ausente e limpa metadados de tag obsoletos em um único script,
  então uma sobrescrita com tag mais nova não consegue mais aterrissar entre
  a escrita condicional e a limpeza que a segue.

- **Dados de sessão de uso único do Inertia sobrevivem a uma resposta que
  falhou.** O `SessionMiddleware` envelhece `_flash.new.*` para
  `_flash.old.*` antes de o handler rodar, então uma resposta que falhou
  enquanto era construída deixava esses valores para serem excluídos pela
  passagem de envelhecimento da solicitação seguinte - o usuário perdia os
  erros de validação ou a mensagem de flash que explicava o que deu errado.
  Um guard com escopo de solicitação agora os reenvia ao flash em toda saída
  sem commit, cancelamento incluído, e só os remove depois de a resposta
  completa ter sido construída.

- **O fan-out de transmissão espera o backend dizer que escreveu.** O
  `SeaProducer::send` só enfileira, e o future retornado é o que reporta a
  escrita de fato no backend, então uma entrega era reportada como enviada
  quando só tinha sido enfileirada. Todo envio de uma passagem agora tem o
  recibo dele aguardado sob um único prazo, o que também impede que um
  broker indisponível mantenha uma solicitação de aplicação aberta
  indefinidamente. Heartbeats de participação seguram o guard de leitura até
  todo heartbeat de snapshot estar enfileirado, então um untrack concorrente
  não pode ser ultrapassado por um heartbeat obsoleto.

- **O `suprnova generate-types` nunca deixa para trás um artefato obsoleto
  ou truncado.** Uma varredura que terminava cedo sobrescrevia a saída com o
  que quer que tivesse, então uma falha de parse transitória excluía em
  silêncio definições de tipo que a aplicação ainda usava. A geração de
  arquivos agora recusa sobrescrever um artefato depois de uma varredura
  incompleta, escreve atomicamente através de um arquivo temporário irmão
  livre de colisão com retentativas limitadas, e pula a escrita por completo
  quando o conteúdo está inalterado. Um symlink de saída mantém o
  comportamento anterior dele: o alvo resolvido é substituído atomicamente e
  o link fica no lugar; um destino somente leitura que mudou é recusado.

- **Duas falhas de comando do `suprnova` são reportadas em vez de
  engolidas.** O `suprnova new` reporta um `git init` que falhou em vez de
  apresentar um projeto como totalmente criado, e o `suprnova
  workflow:install` valida o caminho da migração antes de criar qualquer
  diretório, então um caminho inválido falha sem deixar para trás uma árvore
  feita pela metade.

- **Um savepoint e o registro de efeitos diferidos dele concordam sobre
  identidade.** O statement de savepoint e a marca do registro usavam a
  grafia original do chamador, mas os nomes são insensíveis a maiúsculas e
  minúsculas e o PostgreSQL além disso trata como iguais os nomes que
  compartilham os primeiros 63 bytes ASCII, então um `ROLLBACK TO` podia
  desfazer um savepoint cujos efeitos diferidos o registro tinha arquivado
  sob outra chave. Os dois agora compartilham uma identidade única, validada
  e canônica para o backend. O limite aceito de 64 bytes da API está
  inalterado.

- **O `suprnova <command> --help` imprime a ajuda em vez de rodar o
  comando.** A CLI declarava a própria flag `help` e a consultava somente
  quando nenhum subcomando tinha sido dado, então todo outro `--help`
  imprimia o banner e depois executava o comando mesmo assim. O `suprnova
  migrate:fresh --help` derrubava toda tabela do banco de dados, e a
  salvaguarda que o teria impedido só recusa em produção, enquanto o
  `APP_ENV` tem `local` como padrão. O clap agora é dono do `-h` e do
  `--help` no nível superior e em todo subcomando, em qualquer ordem de
  argumentos, e o banner curado continua sendo o que o nível superior
  imprime. Um teste enumera os subcomandos a partir do próprio clap, então
  um subcomando acrescentado depois fica coberto sem ninguém lembrar de
  acrescentá-lo.

- **Um frontend com scaffold não fixa mais o token CSRF que leu no boot.**
  Os pontos de entrada gerados de Vue, Svelte e React liam o `<meta
  name="csrf-token">` uma vez no carregamento do módulo e anexavam aquele
  valor a toda visita Inertia. Fazer login rotaciona a sessão, o token
  capturado fica obsoleto, e a visita seguinte que muda estado - tipicamente
  o logout - era recusada com `419 CSRF token mismatch`. Toda aplicação
  gerada era distribuída com isso. O hook se foi: o cliente Inertia lê o
  cookie `XSRF-TOKEN` que o `CsrfMiddleware` define e o ecoa de volta em
  `X-XSRF-TOKEN` por conta própria, uma vez por solicitação, então o valor
  que viaja é o que o navegador tem naquele momento. A verificação do lado
  do servidor está inalterada e os dois nomes de header continuam aceitos. O
  capítulo do manual e a documentação do módulo `suprnova::csrf` nomeavam o
  hook antigo como a coisa a fazer; os dois agora o nomeiam como a coisa a
  evitar.

- **O cookie XSRF de uma aplicação com scaffold é utilizável sobre HTTP
  local.** O `CsrfMiddleware::new()` define o cookie `XSRF-TOKEN` legível
  por JS como `Secure` por padrão, enquanto o `env.example` gerado define
  `SESSION_SECURE=false` para desenvolvimento, então um navegador não
  armazenaria nem devolveria o cookie sobre `http://localhost` e toda
  solicitação que muda estado em desenvolvimento era recusada com `419`. O
  bootstrap gerado agora passa o `SessionConfig` dele para o
  `CsrfMiddleware::with_session_config`, que copia o `Secure`, o `SameSite`,
  o `Domain`, o `Path` e o tempo de vida do cookie de sessão para o cookie
  XSRF, para que os dois não possam se separar. Nada é enfraquecido: a
  validação de token, a política de origem padrão e as salvaguardas de
  produção estão inalteradas.

- **O `suprnova make:command` aparece na tela de ajuda.** O comando faz o
  scaffold de um comando de console em `src/commands/` e não tinha linha
  nenhuma na tela curada, então a única forma de saber que ele existia era
  ler o código-fonte. A tela agora é conferida contra a lista de subcomandos
  que o clap reporta, nos dois sentidos, então nem uma linha faltante nem
  uma linha que não nomeie comando nenhum consegue sobreviver.

### Atualizando

- **A maioria das aplicações não precisa de mudança de código nenhuma.** O
  Live e o RenderCache são os dois opt-in: o `suprnova::live` não faz nada
  até um router chamar o `Router::try_live()`, e o `suprnova::render_cache`
  não faz nada até uma rota ser incluída e o `RenderCache::install` ser
  chamado. Todo o resto neste release é uma correção de um comportamento que
  você já tinha. A versão é 2.0.0 porque a superfície do framework cresceu
  em dois subsistemas inteiros e por causa das mudanças de comportamento
  específicas listadas abaixo, e não porque a API comum de aplicação tenha
  sido rearranjada: nenhum item público do crate do framework, do crate de
  macros ou dos crates adaptadores de pagamento, do Magnetar e do Web Push
  foi removido ou renomeado.

- **Tempos de rebuild e pegada de dependências sobem.** O framework agora
  depende do crate de motor interno `suprnova-live` e do `askama` em toda
  build, porque o `suprnova::live`, o `suprnova::render_cache` e o
  `suprnova::view` são módulos incondicionais em vez de features. Não há
  nada a habilitar nem nada a desabilitar.

- **Uma aplicação que nunca usa o RenderCache paga uma sondagem de schema
  por processo.** O `RENDER_CACHE_ENABLED` tem `true` como padrão, então a
  primeira escrita de ORM que um processo faz fora de uma transação pergunta
  uma vez se a migração do RenderCache está presente; ao não encontrá-la,
  aquele processo fica fechado pelo resto da vida dele e não emite mais SQL
  nenhum do RenderCache. Defina `RENDER_CACHE_ENABLED=false` para pular até
  esse único statement. A sondagem nunca roda na transação de um chamador,
  então ela não consegue envenenar uma escrita que você esteja fazendo.

- **Quem constrói um `WebPushClient` a partir de um `reqwest::Client` já
  construído precisa agir.** O `WebPushClient::with_client` agora recusa
  enviar sob o `EndpointPolicy::Strict` padrão e retorna
  `WebPushError::UnconfinedRedirects`, porque a política de redirecionamento
  de um cliente já construído não pode ser inspecionada e o reqwest segue
  redirecionamentos por padrão. Mude para o
  `WebPushClient::with_client_builder`, que honra toda opção de transporte
  que você estava definindo (proxy, TLS, timeouts) e força os
  redirecionamentos a ficarem desligados, ou chame o
  `WebPushClient::allow_unconfined_redirects` se você sabe que o seu cliente
  é seguro. O `WebPushClient::new` não é afetado.

- **URLs assinadas emitidas antes da atualização que carregam uma barra
  final param de verificar.** A assinatura e a verificação as duas cortavam
  uma barra final antes de fazer o hash; as duas agora fazem o hash do
  caminho exatamente. Uma URL assinada como `/orders/1` continua verificando
  em `/orders/1`, e somente em `/orders/1` - um proxy que acrescenta uma
  barra agora produz `SignatureVerdict::Invalid`. Se um proxy ou um
  framework à frente da sua aplicação normaliza caminhos acrescentando uma
  barra, assine a URL do jeito que a solicitação vai chegar.

- **Um agendamento com duas tarefas de mesmo nome agora falha no registro.**
  O `Schedule::add` entra em panic diante de uma duplicata, mantendo a
  tarefa já registrada. Renomeie uma das duas, ou passe para o
  `Schedule::try_add` e trate o erro. Os nomes são exatos e sensíveis a
  maiúsculas e minúsculas.

- **Um lease de workflow mais curto que dois segundos é limitado ou
  recusado.** Um `WORKFLOW_LOCK_TIMEOUT_SECS` abaixo de `2` é limitado a `2`
  com um aviso que nomeia o motivo; um `WorkflowConfig` construído em código
  com um lease mais curto falha no `validate`. Se você estava rodando um
  lease de um segundo deliberadamente, o heartbeat não conseguia renová-lo
  antes de ele expirar.

- **Uma aplicação MySQL que usa workflows deveria acrescentar uma
  migração.** A
  `suprnova::workflow::migrations::NormalizeWorkflowDateTimesForMysql`
  converte as colunas de data de `workflows` e de `workflow_steps` de
  `TIMESTAMP` para `DATETIME`, que é o que o `chrono::NaiveDateTime` de fato
  armazena. Ela é aditiva, e um no-op no PostgreSQL e no SQLite. O MySQL
  pode reconstruir e travar as duas tabelas enquanto a aplica, então
  agende-a de acordo. Um projeto criado pelo `suprnova new` já a recebe
  ligada automaticamente.

- **O `AuthMiddleware` agora resolve o usuário em toda solicitação
  protegida.** Ele costumava aceitar a presença de um identificador
  persistido. A mudança de comportamento é que uma sessão pertencente a um
  usuário excluído ou excluído por soft delete para de autorizar. O custo é
  menor do que parece: o usuário resolvido fica em cache pelo resto da
  solicitação, então um handler que já chamava o `Auth::user()` não paga
  nada a mais e a busca simplesmente saiu do handler para o middleware. Só
  uma solicitação protegida cujo handler nunca resolvia o usuário ganha uma
  busca no provider que não fazia antes. Uma aplicação sem provider de
  usuários vinculado mantém o caminho só de identificador inalterado. O
  `BasicAuthMiddleware` na forma não stateless dele agora exige que o guard
  que ele nomeia exista e seja um guard stateful.

- **O `destroy_all_for_user` custa mais e revoga mais.** Ele agora lê as
  linhas de sessão sobreviventes e compara as identidades de guard dentro de
  cada payload, além da correspondência com o `user_id` indexado, então ele
  alcança sessões autenticadas apenas através de um guard nomeado. A
  revogação é rara o bastante para que a correção tenha sido escolhida em
  vez do uso de índice; se você o chama em um caminho quente, vale saber
  disso.

- **Um `POST` ou um `PATCH` repetido depois de um erro de transporte agora
  precisa do `retry_non_idempotent`.** O ramo do `5xx` já o exigia; o ramo
  de erro de transporte não. Se você contava com uma conexão caída ser
  repetida para uma solicitação não idempotente, opte por isso
  explicitamente.

- **Um processo de produção com um `APP_KEY` ausente ou malformado agora
  falha no `#[suprnova::main]`, e não no `Server::from_config`.** Isso
  inclui um binário de console ou um ponto de entrada de worker que nunca
  constrói um servidor. Ambientes local, de desenvolvimento e de teste
  continuam gerando uma chave transitória e avisando. O `APP_PREVIOUS_KEYS`
  é aceito como alias para o `APP_KEY_PREVIOUS`; se os dois estiverem
  definidos com valores diferentes, o nome Suprnova vence e o duplicado é
  nomeado em um aviso sobre o qual você deveria agir.

- **Um worker apontado para uma conexão de fila `failover` agora drena toda
  conexão da lista.** Se você estava rodando um segundo worker contra uma
  conexão de fallback diretamente - o que as notas da 1.3.3 mandavam fazer
  -, aquele worker e o worker de failover vão estar os dois drenando ela.
  Remova o worker extra, ou mantenha-o e aceite a competição. Os contadores
  e as listagens agora agregam toda conexão, então um dashboard lendo
  `pending_size` na conexão de failover vai reportar um número maior do que
  reportava.

- **Implementações customizadas de armazenamento e de driver ganham padrões
  que falham fechado, nunca comportamento enfraquecido.** Os armazenamentos
  de token de provider, de cerimônia e de remember do Magnetar ganharam
  métodos para as operações atômicas descritas acima; cada um tem um padrão
  que recusa em vez de executar a operação de forma não atômica, então uma
  implementação externa ainda compila, mas vai reportar falha até
  implementar o método. O `WebhookHandler::mirrors_payment_transactions` e o
  `try_extract_payment_snapshot`, o `QueueDriver::queue_filter_capability` e
  o `GlobalScope::dependency` têm todos padrões que preservam exatamente o
  comportamento anterior.

- **Para adotar o Live**, vincule um registro durante o bootstrap com
  `App::singleton(crate::live::registry().expect("Live component
  registry"))`, instale as rotas reservadas com o `Router::try_live_with` e
  um guard carregando o seu `AuthMiddleware`, o `LiveTenantMiddleware` e o
  `RateLimitMiddleware`, e registre o `CsrfMiddleware::new()` globalmente -
  o Live verifica a própria prova de origem, então você não precisa alargar
  o `OriginPolicy` para a aplicação inteira, e se você o alargou por outro
  motivo, estreite-o de volta. Use o `AuthMiddleware::optional()` no guard
  do Live se você quiser que visitantes anônimos ajam sobre seeds públicos;
  o `AuthMiddleware::new()` responde `401` para toda solicitação anônima
  antes de qualquer trabalho do motor. Depois rode o `suprnova live:make` e
  o `suprnova live:check`. O `manual/live.md` tem o passo a passo completo.

- **Para adotar o RenderCache**, acrescente a
  `suprnova::render_cache::migration::Migration` ao seu `Migrator` (e também
  a `suprnova::render_cache::migration::TierMigration` se você roda o perfil
  `database` ou `redis`), inclua rotas e grupos com o
  `Router::try_render_cache` e o `Router::try_render_cache_group`, e termine
  com o `RenderCache::install`. Como o `install` é assíncrono - ele sonda as
  tabelas do ledger antes de montar um runtime -, o router tem de ser
  construído através do `Application::try_routes_async` em vez do
  `try_routes`. A instalação tem de vir depois de todo middleware que
  estabelece locale, sessão ou identidade com escopo de solicitação, e
  depois de toda rota e todo grupo terem sido incluídos. Um perfil
  compartilhado recusa dar boot sem a migração de camada dele ou com um
  endpoint que ninguém responde, o que é deliberado.

- **Defina o `APP_BUILD_ID` uma vez por deploy.** Ele é misturado em toda
  chave de busca do RenderCache, então mudá-lo é o que impede uma build nova
  de servir entradas que a anterior publicou. O padrão dele é a versão de
  pacote do crate da sua aplicação, que não muda quando você publica um
  template, uma tradução ou uma correção de handler. Um id de commit serve:
  `APP_BUILD_ID=$(git rev-parse --short HEAD)`.

- **Considere mover o seu `Cargo.toml` para o formato de build de
  produção.** Declare o `suprnova` com `default-features = false` mais os
  nove padrões que não são `testing` que você usa, e reacrescente `features
  = ["testing"]` sob `[dev-dependencies]`. O Cargo puxa as features de uma
  dependência de desenvolvimento apenas para o `cargo test` e outras builds
  `--tests`, então os seus binários distribuídos param de carregar costuras
  de teste enquanto o `cargo test` continua funcionando inalterado. Um
  projeto criado pelo `suprnova new` já está nesse formato; o
  `manual/deployment.md` o documenta para uma aplicação existente.

- **Se você consumia o runtime `@suprnova/live` autônomo ou escreveu o
  próprio host de assinaturas, o descritor de evento registrado mudou.** O
  `DESCRIPTOR_SCHEMA_VERSION` passou de 1 para 2 e os campos de evento
  registrado do descritor agora são `maximum_hops`, `maximum_fanout` e
  `payload_contract`, em vez de `maximumHops`, `maximumFanout` e
  `payloadContract` - eles eram as únicas chaves em camelCase em um contrato
  JSON público que é snake_case em todo o resto. Um descritor assinado na
  versão de schema 1 é recusado com
  `SubscriptionErrorKind::InvalidDescriptor` em vez de ser lido com três
  campos ausentes. Nada em uma aplicação Suprnova 1.3.7 consumia este
  contrato, então para a maioria dos leitores não há nada a fazer.

## 1.3.7 - 2026-08-26

### Adicionado

- **Onde o middleware de página de erro do Inertia fica agora é escolha
  sua, e está documentado.** O `Inertia::install` registra o
  `InertiaErrorPageMiddleware` na posição mais interna da camada Inertia,
  então ele cobre o handler, o middleware de rota e tudo o que você
  registra depois dessa chamada - é por isso que o scaffold coloca o
  `CsrfMiddleware` abaixo dela. Ele não cobre nada registrado *acima* da
  chamada, porque um middleware que responde sem chamar `next` entrega a
  sua resposta a nada que esteja registrado dentro dele. O caso que
  realmente pega é uma sessão expirada enviando um formulário: o
  `CsrfMiddleware` registrado acima da instalação responde `419` com
  `{"message":"CSRF token mismatch."}` e o usuário recebe o modal de falha
  do Inertia justamente no fluxo que ele tem mais chance de encontrar; o
  `429` de um limitador de taxa mais externo e o `401` de um guard de
  autenticação são a mesma coisa. Registrar o middleware você mesmo, mais
  para fora, já funcionava na 1.3.6 - o tipo era público e o registro é
  idempotente por tipo, então um registro anterior mantinha o seu lugar -
  mas nada dizia isso e nada no `install` reconhecia o caso, o que fazia
  disso um acidente em vez de um contrato. Agora é um contrato: registre
  `InertiaErrorPageMiddleware::new("Error")` depois do `SessionMiddleware`
  e do `LocaleMiddleware` e antes do middleware cujas recusas ele deve
  cobrir, e o `install` procura por ele, loga em `debug` e pula o próprio.
  O componente que você nomeou nesse registro é o que é renderizado, então
  você nomeia a página uma vez e o `.error_page(...)` da config passa a ser
  opcional - ele continua sendo o que faz o `install` registrar um
  middleware para um app que não coloca um por conta própria. As duas
  regras de ordem estão documentadas no tipo e no manual.

### Corrigido

- **Uma página com SSR tem um único `<title>`, e ele é o da própria
  página.** O shell HTML escrevia o seu `default_title` e depois o head do
  worker de SSR na íntegra, então toda página que renderizava um título
  através do componente `Head` do Inertia produzia um documento com dois
  elementos `<title>` e o genérico do framework em primeiro lugar. O
  primeiro é o que a aba do navegador, o crawler e a prévia de link leem,
  então o título real nunca aparecia. Um head de worker que carrega um
  título agora substitui o título do shell em vez de se somar a ele -
  tanto o `default_title` quanto um `InertiaResponse::title(...)` por
  resposta ficam de fora; um head sem título deixa o título do shell
  exatamente onde ele estava.
- **O documento declara o idioma em que está escrito.** O shell fixava
  `<html lang="en">` no código, então um leitor que trocasse para o
  japonês recebia prosa em japonês num documento que se dizia em inglês -
  um leitor de tela escolhe a sua voz a partir desse atributo e um
  mecanismo de busca o toma como o sinal de idioma da página. Agora ele
  carrega o locale em vigor para a solicitação: o que o `LocaleMiddleware`
  detectou, depois um override de `Lang::set_locale`, depois o
  `APP_LOCALE` configurado, na mesma forma BCP 47 que o `Locale` renderiza
  (`pt-BR`, `zh-Hans`). Isso vale também para a página de erro, que é
  renderizada na saída e foi o caso que revelou o problema. Sem a feature
  `localization`, o shell mantém `en`.

### Atualizando

- Nada é obrigatório. As duas correções valem para todo app Inertia na
  atualização, e o `Inertia::install` se comporta exatamente como antes
  para um app que não registra o middleware de página de erro por conta
  própria.
- Um app que injetava `<html lang="...">` no documento já pronto com um
  middleware próprio pode apagá-lo - o shell faz isso agora, a partir do
  mesmo locale que aquele middleware lia.
- Um app cujo `CsrfMiddleware`, limitador de taxa ou guard de autenticação
  esteja registrado **antes** do `Inertia::install` deve registrar
  `InertiaErrorPageMiddleware::new("Error")` depois do `LocaleMiddleware`
  e antes desse middleware, para que as recusas dele renderizem a página
  de erro em vez de chegar ao cliente como JSON cru. O `install` então
  deixa de adicionar o próprio, e o componente que você nomeou no registro
  é o que é renderizado, então o `.error_page("Error")` da config é
  opcional - mantenha ou remova. O `bootstrap.rs` com scaffold registra o
  CSRF depois da instalação, então um projeto gerado pelo `suprnova new`
  não precisa de mudança nenhuma.
- Um app que renderiza o próprio `<title>` através do componente `Head` do
  Inertia sob SSR vai ver o título do shell parar de aparecer no
  documento - tanto o `InertiaConfig::default_title` quanto um
  `InertiaResponse::title(...)` por resposta. Essa é a correção: o título
  da própria página é o único do documento. Se você contava com o título
  do shell como prefixo ou sufixo, mova-o para dentro do componente
  `Head`, onde o resto do título vive.

## 1.3.6 - 2026-08-26

### Adicionado

- **Erros do framework podem renderizar a sua própria página Inertia em
  vez do modal de falha do cliente.** Um usuário sem determinada
  permissão clicava num link de navegação para uma rota protegida e
  recebia a tela do Inertia com "All Inertia requests must receive a
  valid Inertia response, however a plain JSON response was received": o
  `403` carregava o corpo de erro JSON do framework e nenhum header
  `X-Inertia`, então o cliente o recusava. O mesmo valia para um `404`
  sem rota, um `429` barrado por limitação de taxa e o `500` de um
  handler que falha. Nomeie um componente de página com
  `InertiaConfig::error_page("Error")` e essas respostas passam a
  renderizar essa página no status original, com as props `status`,
  `message` e - quando o erro carregava um - `request_id`. Todo header
  definido pela resposta de erro sobrevive à troca, exceto os que apenas
  descreviam o corpo que está sendo substituído (`Content-*`,
  `Transfer-Encoding`) ou governavam como ele podia ser armazenado
  (`Cache-Control`, `Expires`, `Age`, `ETag`, `Last-Modified`), então
  `Retry-After` num `429`, `WWW-Authenticate` num `401`, `Vary` e
  `Set-Cookie` continuam todos chegando ao cliente. A página define
  `Cache-Control: no-cache, private` para si mesma: ela carrega as suas
  props compartilhadas, então nunca pode ser armazenada por um cache
  compartilhado e servida a outro visitante, não importa o que a
  resposta que ela substituiu permitisse. Uma visita Inertia recebe o objeto de
  página JSON; uma navegação dura recebe o shell HTML completo, então
  colar a URL na barra de endereços também funciona. Tudo que já tem um
  dono fica intocado: `422`s de validação continuam redirecionando de
  volta ao formulário, bounces de `X-Inertia-Location` e respostas que
  já são páginas Inertia passam direto, e um cliente cujo `Accept`
  prefere JSON mantém exatamente o corpo que recebia antes. O
  `suprnova new` gera `frontend/src/pages/Error.*` por scaffold e define
  `.error_page("Error")`, então projetos novos já estão cobertos sem
  fazer nada.

### Corrigido

- **Um disco local não recusa mais um caminho legítimo porque outra task
  o tocou.** A salvaguarda de caminhos resolvia cada componente de um
  caminho com duas sondagens e combinava as duas num único veredito,
  então atividade concorrente comum podia ser lida como uma fuga por
  symlink: um componente que o `canonicalize` acabara de reportar como
  ausente, e que outra task então criava como um arquivo comum, voltava
  como `PermissionDenied` apontando um symlink que nunca esteve ali.
  O problema pegava com mais força onde escritores disputam por design -
  um `write_with(..).if_not_exists(true)` que perdia a corrida recebia
  essa recusa em vez de `ConditionNotMatch` sempre que o vencedor
  publicava a chave entre as duas sondagens, o que sob uma suíte de
  testes carregada dava aproximadamente um terço das execuções. Cada componente agora é
  classificado a partir de uma única passagem, `symlink_metadata`
  primeiro: nada ali é espaço livre, um arquivo ou diretório comum é
  resolvido e confinado como antes, e só um symlink que ainda assim não
  pode ser resolvido é recusado. Um componente que desaparece no meio da
  classificação é olhado mais uma vez em vez de recusado. Toda recusa de
  symlink permanece inalterada.

### Atualizando

- Nada muda para um app existente até que ele opte por isso. O
  `InertiaConfig::error_page` tem `None` como padrão, e o
  `Inertia::install` registra o middleware de página de erro somente
  quando um componente é nomeado, então as respostas de erro mantêm
  exatamente os seus corpos. Para adotá-lo, adicione um componente de
  página chamado `Error` ao lado dos outros (ele recebe `status`,
  `message` e um `request_id` opcional) e encadeie
  `.error_page("Error")` no `InertiaConfig` que você passa para
  `Inertia::install`. Um handler que sofre **panic** continua fora de
  escopo: a rede de captura de panic envolve toda a cadeia de
  middlewares, então o `500` sintetizado por ela é construído depois que
  todo middleware já desempilhou. Retorne `Err(...)` em vez de sofrer
  panic e a página de erro cobre o caso. Note que o critério é o
  **formato** do corpo, não quem o escreveu: num status de erro, um
  corpo vazio, um objeto JSON cujo `message` é uma string e o texto
  `404 Not Found` do próprio roteador são reescritos não importa qual
  middleware os construiu, e só `message` e `request_id` sobrevivem até
  as props. Uma resposta que precisa manter o próprio corpo JSON deve
  usar para o seu texto uma chave diferente de `message`, ou definir
  `X-Inertia: true` em si mesma. E registre o `LocaleMiddleware`
  **antes de** `Inertia::install`: a página de erro é renderizada na
  saída, depois que todo middleware registrado dentro da camada Inertia
  já retornou, então um escopo de locale aberto lá dentro já não existe
  mais e toda página de erro seria renderizada no locale padrão do app.
  O `bootstrap.rs` com scaffold já faz isso, e o mesmo raciocínio vale
  para qualquer middleware seu com escopo de solicitação cujo estado as
  props compartilhadas da página leiam.

## 1.3.5 - 2026-08-26

### Alterado

- **Toda seção do changelog é legível nas seis traduções do manual.** Os
  manuais em de, es, fr, ja, pt-BR e zh-Hans traziam as seções de 1.3.0 a
  1.3.2 em inglês, atrás de uma nota do tradutor, e seções mais antigas
  com linhas soltas em inglês; toda seção de 1.3.5 até 0.1.0 agora está
  traduzida, e as notas sumiram.

### Corrigido

- **Discos de sistema de arquivos local publicam todo objeto em um único
  passo.** O `Storage::register_fs` e o `register_fs_with` agora preparam
  `disk.write(...)`, `disk.writer(...)` e `disk.copy(...)` como um arquivo
  temporário em `<root>/.suprnova-atomic/` e o publicam no alvo com um
  único `rename(2)`, então nenhum deles jamais é observável com um tamanho
  parcial. Antes disso, o driver abria o alvo com `create + truncate` e
  transmitia para dentro dele no lugar: um leitor concorrente recebia um
  objeto vazio ou escrito pela metade durante toda a duração da escrita, e
  um crash no meio da escrita deixava um objeto truncado no caminho ativo.
  O `abort()` em um writer agora descarta o arquivo preparado em vez de
  falhar com `Unsupported`.
- **`write_with(..).if_not_exists(true)` é uma criação exclusiva de
  verdade em um disco local.** Ela é publicada com `link(2)`, que falha
  atomicamente no kernel quando o alvo existe, então exatamente um entre
  qualquer número de chamadores concorrentes tem sucesso e todos os outros
  recebem `ConditionNotMatch` sem ter escrito nada. Uma escrita preparada
  publicada por um rename simples teria degradado a condição para uma
  checagem seguida de uma sobrescrita, descartando silenciosamente todos
  os writers menos o último - que é o oposto do motivo pelo qual se
  recorre a essa primitiva.
- **Um `append` que cria o objeto continua sendo um append.** Appends são
  a única operação feita no lugar em um disco local, e agora isso vale
  também para o primeiro deles, então dois writers dando append no mesmo
  objeto ausente aterrissam os dois, em vez de um deles preparar a própria
  cópia e sobrescrever o outro.

- **O `suprnova serve` não recompila mais um projeto em que ninguém tocou,
  e o `suprnova generate-types --watch` também não.** Os dois monitores
  classificavam um evento de sistema de arquivos apenas pelo caminho, e o
  gerador lê todo arquivo `.rs` dentro da mesma árvore `src/` que eles
  monitoram - então, no Linux, onde o kernel reporta essas leituras, cada
  regeneração agendava a seguinte. Um projeto recém-criado com scaffold
  regenerava seus tipos e reiniciava seu backend a cada meio segundo, para
  sempre, sem uma única edição de código-fonte. Agora só contam eventos
  que significam que os bytes em disco realmente mudaram. O
  `generate-types --watch` também não tinha debounce nenhum, então agia no
  primeiro arquivo de uma rajada em vez de no último; ele agora
  compartilha a borda de descida de 500 ms do `serve`, e os dois monitores
  compartilham uma implementação só, para que a próxima correção não possa
  chegar em apenas um deles. O gerador compara antes de escrever, então
  uma regeneração cuja saída é byte a byte idêntica deixa o arquivo, e o
  mtime dele, em paz.

- **O monitor do backend está delimitado aos caminhos a partir dos quais o
  servidor é construído.** O `cargo watch` rodava sem nenhum `-w`, então
  monitorava o projeto inteiro fora do gitignore: salvar um componente
  Svelte, ou regenerar `frontend/src/types/inertia-props.ts`, recompilava
  o framework e reiniciava o servidor. Agora ele monitora `src/`, `cmd/`,
  `Cargo.toml`, `Cargo.lock`, `.env` e `lang/` - as entradas da build mais
  as duas árvores lidas uma única vez no boot - cada um incluído somente
  quando existe, já que o cargo-watch recusa um caminho `-w` que não
  exista. É em `cmd/` que o scaffold full-stack mantém o `main.rs` do
  binário do servidor. A invocação também passa `--no-vcs-ignores`, porque
  o cargo-watch aplica o `.gitignore` a raízes `-w` nomeadas
  explicitamente e o scaffold coloca `.env` no gitignore, o que deixaria
  `-w .env` monitorando nada; o `-w` já estreitou a superfície, então a
  flag não tem como alargá-la. Edições no frontend e arquivos `.ts`
  gerados não reiniciam mais o backend.

- **`serde_json::Value` é gerado como `JsonValue` em vez de `unknown`.**
  Ele degradava para `unknown` e avisava que "não é um struct que este
  projeto define", um conselho errado para um documento JSON - e as
  próprias páginas de login e de cadastro do scaffold tropeçavam nisso
  duas vezes a cada regeneração, então todo projeto novo já vinha de
  fábrica emitindo aviso. Agora ele emite um alias recursivo `JsonValue`,
  declarado uma única vez no topo do arquivo gerado e só quando algo o
  referencia. Um `Value` puro também mapeia para lá, a menos que o projeto
  defina um struct `Value` próprio.

- **Nem o `generate-types` nem o `serve` reportam como gerado um arquivo
  que não escreveram.** Como uma passagem agora só escreve quando o
  conteúdo emitido difere, `Generated <path>` era uma afirmação sobre o
  sistema de arquivos que era falsa em toda reexecução de um projeto
  inalterado. O `generate-types` diz `<path> is up to date` em vez disso,
  tanto na execução única quanto no `--watch`, e a passagem de
  inicialização do `serve` diz `N type(s) up to date → <path>`, mantendo a
  contagem. O monitor de arquivos do `serve` agora fica calado em uma
  regeneração que não escreveu nada, tanto em texto quanto sob `--json`:
  um evento `types_regenerated` significa que o arquivo gerado em disco
  está diferente agora, então silêncio depois de um salvamento diz a você
  que sua edição não mudou nenhuma forma de prop.

### Atualizando

- **`.suprnova-atomic` é reservado na raiz de todo disco local.** O
  diretório de preparo tem que viver dentro da raiz - um irmão da raiz
  pode estar em um sistema de arquivos diferente quando a raiz é um ponto
  de montagem, e todo rename falharia com `EXDEV` -, então o nome é
  reservado em vez de meramente convencional. Qualquer caminho cujo
  primeiro componente seja `.suprnova-atomic` agora é recusado com um erro
  de permissão (leitura, escrita, exclusão, stat e listagem igualmente),
  assim como qualquer caminho que resolva para dentro do diretório por um
  symlink, e a entrada é filtrada para fora de `files`, `directories`,
  `all_files` e `all_directories`. Se a raiz de um disco seu já contiver
  uma entrada `.suprnova-atomic` própria, ela não é mais alcançável
  através daquele disco: mova-a para outro lugar antes de atualizar. Um
  arquivo comum com esse nome é recusado já no registro, com uma mensagem
  dizendo isso, em vez de falhar mais tarde dentro do driver. O nome é
  exportado como `suprnova::ATOMIC_STAGING_DIR` para que ferramentas de
  backup e de sincronização possam deixá-lo de fora.
- **Publicar por rename substitui o inode do alvo.** Reescrever um objeto
  em um disco local não preserva mais o modo, o dono nem os hard links
  dele, e um leitor que segura um descritor aberto continua com o conteúdo
  antigo em vez de ver os bytes novos. Esse é o custo padrão da publicação
  atômica, mas é uma mudança de comportamento se você contava com qualquer
  um dos dois.
- **Uma escrita condicional precisa de um sistema de arquivos com hard
  links.** O `if_not_exists` é publicado com `link(2)`, que não é
  suportado em FAT, exFAT e alguns sistemas de arquivos de rede. Ali ele
  falha de vez, em vez de cair para uma checagem seguida de uma
  sobrescrita, porque um fallback te entregaria uma garantia de
  exclusividade que não se sustenta. Nada mais no disco é afetado.
- **Um primeiro `append` que falha deixa um objeto vazio.** Um append é a
  única operação que não é publicada em um único passo, então o objeto é
  criado antes de os bytes chegarem; um primeiro append que falha ou é
  abortado o deixa para trás, exatamente como um append sobre um objeto
  existente sempre deixou.
- **Um symlink quebrado na raiz do disco é recusado, não sobrescrito.** Um
  caminho cujo alvo do symlink não existe não pode mais ser escrito, nem
  receber append, nem ser copiado por cima, nem ser movido por cima, nem
  ser excluído através do disco. O `1.3.4` substituía um link desses por
  um arquivo comum; a guarda não tem como provar para onde um link
  irresolvível leva, e criar através de um cria o alvo dele em qualquer
  lugar do host, então agora ela recusa. Remova o link fora do disco se
  você realmente queria escrever ali.
- **Nada varre o diretório de preparo.** Ele guarda arquivos temporários
  em voo mais o que um processo que morreu no meio de uma publicação
  deixou para trás, então um host em crash loop o faz crescer sem limite.
  Esvaziá-lo enquanto nada estiver escrevendo no disco é seguro; deixá-lo
  de fora dos backups é recomendado.

## 1.3.4 - 2026-08-25

### Adicionado

- **Discos read-through ganham uma flag `copy` e resolvem `copy` / `rename`
  através do fallback.** Defina `copy: false` no `ReadThroughConfig` para
  servir acertos do fallback sem escrevê-los de volta, o que transforma o
  disco em uma sobreposição transparente e estreita cada busca ao range que
  você pediu. `copy` e `rename` agora transmitem para o destino primário uma
  origem que vive apenas no fallback; um `rename` também exclui a origem no
  fallback, então uma leitura posterior não consegue ressuscitar o objeto
  movido. As condições atravessam esse caminho de streaming: `if_not_exists`
  continua recusando um destino existente, a versão de origem de uma cópia
  seleciona qual objeto o fallback entrega, e o `if_match` de uma cópia é
  recusado com `Unsupported` em vez de ser descartado em silêncio. Uma
  transferência que falha no meio remove apenas um destino que ela própria
  criou, então não consegue destruir um objeto que já estava lá.
- **Jobs com debounce e listeners em fila com debounce.** O
  `Job::debounce_for()` colapsa uma rajada de dispatches em uma execução, uma
  janela depois do mais recente, carregando o payload mais novo. É o espelho
  do `push_unique`, que mantém o primeiro dispatch e suprime o resto. O
  `Job::max_debounce_wait()` impede que uma rajada contínua adie o trabalho
  para sempre, e o `Job::debounce_id(&self)` delimita a janela por entidade,
  de modo que vinte atualizações em um pedido colapsam sem tocar nas de outro
  pedido. O `Queue::push_debounced(job, DebounceOptions)` define a janela no
  local de chamada, e o `DebouncedListener::new(window, build).keyed_by(...)`
  aplica debounce a um listener de evento com a chave derivada do evento - um
  `QueuedListener` comum já honra uma janela que o próprio job declara. Todo
  dispatch continua sendo enfileirado; o colapso é resolvido no worker, que
  reconhece um envelope substituído e emite `JobDebounced`. O debounce falha
  em aberto: uma janela expirada ou despejada executa o job em vez de
  descartá-lo. Cada execução de fato inicia uma janela de espera máxima nova,
  então uma rajada sempre mede a espera máxima dela a partir do próprio
  primeiro dispatch, em vez de herdar a da rajada anterior. Um job não pode
  declarar `debounce_for` e `unique_id` ao mesmo tempo, e chains e batches
  recusam um job com debounce - um elo substituído deixaria o resto da chain
  encalhado, e um job de batch substituído deixaria a contagem de pendentes do
  batch acima de zero para sempre. O envelope carrega dois campos aditivos
  para isso e continua byte a byte idêntico na rede para todo push sem
  debounce.

- **O `Storage::register_read_through` compõe dois discos em um disco
  read-through.** Leituras e metadados resolvem contra o primário primeiro e
  recaem para o segundo disco; qualquer coisa encontrada no fallback é escrita
  de volta no primário, então uma migração de store se completa sob tráfego
  real. Escritas e listagens ficam no primário, e um delete remove o objeto
  dos dois discos. Defina `throw_on_promotion_failure` quando uma promoção que
  falhou tiver de aparecer em vez de degradar para uma leitura no fallback.
  Uma promoção é publicada atomicamente, então nenhum leitor consegue ver um
  objeto escrito pela metade, e ela leva junto o content type, o cache
  control, o content disposition, o content encoding e os metadados de usuário
  do objeto no fallback. Uma leitura versionada ou condicional é repassada com
  a condição dela intacta e servida sem ser promovida.
- **O `Queue::forward` redireciona uma fila inteira por nome.** Onde o
  `Queue::route` é chaveado por tipo de job, o `Queue::forward("default",
  "high")` é chaveado por nome de fila - a alavanca para aposentar um pool,
  absorver um backlog ou tirar trabalho de um pool que você está prestes a
  derrubar, sem tocar em um único job ou rota. Ele se aplica dos dois lados:
  novos pushes que resolveram para `default` aterrissam em `high`, *e* um
  worker iniciado com `--queue=default` drena `high`, então o destino não
  consegue juntar trabalho que ninguém reivindica. Encaminhar `default` pega
  os jobs que não nomearam fila nenhuma. Um forward é uma única busca, nunca
  uma cadeia, então uma permuta (`a -> b` com `b -> a` também registrado) ou
  uma rotação mais longa é uma troca coerente de pools, e não um laço -
  exatamente como no Laravel, cujo resolvedor é essa mesma busca única. A
  pausa continua sendo avaliada sobre os nomes com que um worker foi iniciado,
  então o `Queue::pause(&connection, "default")` para aquele worker mesmo
  enquanto `default` está encaminhada. O `Queue::forward_on(from, to,
  connection)` restringe um forward a um nome de conexão, comparado com o nome
  de conexão deste processo, e não com a conexão declarada de um job, de modo
  que as duas metades do redirecionamento se condicionam ao mesmo valor. O
  `Queue::forward_for(from)` lê um forward de volta, e o `Queue::try_forward`
  é o irmão falível. As chamadas de inspeção (`Queue::pending_jobs` e os
  irmãos dela) deliberadamente não seguem um forward, então um backlog deixado
  para trás em uma fila encaminhada continua visível.

- **Comandos Redis com formato de leitura tentam de novo diante de uma falha
  transitória em vez de expô-la.** O gerenciador de conexões já reconectava em
  segundo plano, mas o comando que atingiu o socket morto ainda fazia a sua
  chamada falhar. `GET`, `EXISTS`, as páginas de `SCAN` e `SSCAN` por trás de
  `Cache::flush` / `Cache::flush_tags`, as leituras de `XLEN` / `ZCARD` /
  `XPENDING` do driver de filas e o cálculo de `Retry-After` do limitador de
  taxa agora tentam de novo uma vez, depois de uma pausa curta. O
  `REDIS_COMMAND_RETRIES` acrescenta mais retentativas por cima, com limite
  de 10. Faça o orçamento da retentativa em segundos, e não em milissegundos: a
  segunda tentativa espera pela conexão substituta, então ela custa todo o
  orçamento de conexão e de resposta do driver, e um comando que sofre timeout
  conta como transitório tanto quanto um que foi derrubado. Escritas nunca
  tentam de novo, em nenhuma configuração: um erro transitório significa que a
  conexão falhou, não que o servidor recusou o comando, então repetir um
  `SET`, um `INCR`, uma aquisição de lock, um hit de limitação de taxa ou um
  pop de fila poderia executá-lo duas vezes. As mensagens de erro não mudaram,
  então qualquer coisa que case sobre elas continua funcionando.
- **Um worker pausado agora avisa que está pausado.** O `queue:work` imprime
  uma linha por transição - `2026-08-25 14:03:11 Queue billing PAUSED`, e
  `RESUMED` na volta - e o worker emite `WorkerQueuePaused` /
  `WorkerQueueResumed` para que você possa rotear o mesmo sinal para o seu
  próprio alerting. Esses são o par do lado do worker; os já existentes
  `QueuePaused` / `QueueResumed` disparam no processo que executou o
  `queue:pause`, que nunca é o worker, então até agora um worker que ficava
  quieto porque alguém pausou a fila dele era indistinguível de um travado.
  Cada evento dispara uma vez por transição, e não uma vez por poll. O campo
  `queue` deles é opcional: um worker iniciado sem `--queue` drena tudo e não
  tem nomes de fila para reportar sob um `pause_all`, então ele reporta `None`
  em vez de inventar um nome sobre o qual um listener poderia casar.
- **Caminhos de `?include=` são limitados a cinco segmentos, e o
  `max_relationship_depth` move o teto.** Um grafo de relacionamentos cíclico
  transforma `?include=author.posts.author.posts...` em um fan-out que o
  cliente controla, limitado apenas pela query string. Os caminhos agora são
  truncados enquanto são parseados; chame
  `suprnova::max_relationship_depth(n)` em `bootstrap::register()` para mudar
  o limite, ou passe `0` para desligar os includes.
- **`Gt`, `Gte`, `Lt` e `Lte` comparam um campo contra um número ou contra
  outro campo.** O `CompareWith` nomeia o operando e a medida em um único
  valor: `Number` para um literal, `NumericField` para um irmão numérico, e
  `LengthField` para um irmão comparado por contagem de caracteres. Um
  operando que a regra não consegue medir faz o campo falhar em vez de entrar
  em panic.
- **Três regras de pertencimento entram no conjunto embutido: `InArray`,
  `Contains` e `DoesntContain`.** O `InArray` checa um valor contra a lista de
  outro campo, e você passa a lista diretamente em vez de nomear o campo em
  uma string de regra. `Contains` e `DoesntContain` rodam sobre um array JSON
  e casam um parâmetro apenas contra um elemento do tipo string, então `1` e
  `"1"` continuam distintos.
- **O pool de banco de dados agora tem knobs de vivacidade.**
  `DB_IDLE_TIMEOUT`, `DB_MAX_LIFETIME`, `DB_ACQUIRE_TIMEOUT`,
  `DB_TEST_BEFORE_ACQUIRE` e `DB_PING_AFTER_IDLE` controlam quando o pool
  fecha, recicla e faz ping em uma conexão, com setters correspondentes em
  `DatabaseConfig::builder()`. Cada um vem sem valor definido por padrão,
  então o pool de um deployment existente se comporta exatamente como se
  comportava. Use-os quando um gateway NAT ou um firewall derruba conexões
  ociosas: o sqlx não expõe nenhum equivalente ao `keepalives_*` da libpq,
  então a reciclagem do pool é o mecanismo.
- **O `db:seed <Class>` reporta o progresso dele.** Uma execução direcionada
  imprime uma linha `RUNNING` antes do seeder e uma linha `DONE` com os
  milissegundos decorridos depois dele. Um `db:seed` puro fica em silêncio. O
  formatador, `suprnova::two_column_detail`, está disponível para os seus
  próprios handlers `#[command]`.
- **Relações muitos-para-muitos agora filtram por colunas do pivot.**
  `where_pivot`, `where_pivot_op`, `where_pivot_in`, `where_pivot_not_in`,
  `where_pivot_null`, `where_pivot_not_null`, `where_pivot_between`,
  `where_pivot_not_between`, `where_pivot_group` e os gêmeos `or_` deles
  restringem `get`, `first` e `count` em `BelongsToMany`, `MorphToMany` e
  `MorphedByMany`. O `where_pivot_group` recebe uma closure e renderiza um
  único grupo entre parênteses, então ele permanece atômico dentro de um
  `or_where_pivot` seguinte. Filtros de pivot valem somente para leituras:
  `attach`, `attach_with`, `detach` e `sync` retornam um erro enquanto um
  estiver definido, e o eager loading não os carrega.
- **O `where_binary` compara valores de coluna byte a byte.** A família
  (`where_binary`, `or_where_binary`, `where_not_binary`,
  `or_where_not_binary`) está disponível no `Builder<M>`, e `where_binary` e
  `where_not_binary` estão disponíveis no `DB::table(...)`. MySQL e MariaDB
  emitem `= binary`; PostgreSQL e SQLite retornam um erro quando a consulta é
  renderizada, em vez de recair para uma correspondência dependente de
  collation.
- **O `Builder::try_to_sql_with_bindings_for` renderiza SQL para um dialeto
  sem entrar em panic.** É o irmão falível do `to_sql_with_bindings_for`, para
  os casos em que um builder legitimamente não consegue renderizar para um
  backend.
- **O `Model::refresh_for_update` recarrega uma linha sob um lock
  `FOR UPDATE`.** Chame-o dentro de uma transação quando você precisar do
  estado atual da linha e do lock exclusivo em um só statement. O SQLite não
  tem lock em nível de linha, então lá a cláusula de lock não tem efeito.
- **`Builder::or_where_key` e `Builder::or_where_key_not` acrescentam filtros
  de chave primária como uma disjunção.** Os dois se dobram na cláusula
  `WHERE` anterior do mesmo jeito que o `or_where` faz, e os dois trazem os
  aliases `or_filter_key` e `or_filter_key_not`.
- **O `Builder::in_order_of` ordena as linhas em uma sequência explícita.**
  Passe uma coluna e os valores na ordem que você quer; linhas cujo valor não
  está na lista ficam por último. Os valores são vinculados como parâmetros,
  então podem vir de dados da solicitação com segurança.

### Corrigido

- **O cookie de bypass do modo de manutenção agora expira no servidor.** O TTL
  de 12 horas era um `max-age` que o navegador aplicava, então um cookie
  capturado continuava funcionando até você rotacionar o segredo. O payload
  criptografado agora carrega o prazo, e toda solicitação o reconfere.
- **O `suprnova serve` executa um projeto sem frontend.** Um projeto com
  scaffold feito por `suprnova new --api` não tem diretório `frontend/`, e o
  `serve` o rejeitava com "No frontend directory found. Are you in a Suprnova
  project directory?" a menos que você passasse `--backend-only`. Agora ele
  pula o painel do Vite e a geração de TypeScript que o alimenta, e serve o
  backend. O `--frontend-only` continua falhando num projeto desses, com uma
  mensagem que diz o porquê.

### Atualizando

- **Cookies de bypass emitidos antes deste release param de funcionar.** O
  payload do cookie mudou do segredo puro para um objeto selado
  `{ secret, expires_at }`, e um payload sem prazo é recusado. Visite a URL do
  segredo uma vez depois de atualizar para receber um cookie novo. Nada mais
  muda: `down`, `up`, `--secret` e `--with-secret` se comportam como antes.
- **Um caminho de include com mais de cinco segmentos agora retorna os cinco
  primeiros relacionamentos dele em vez de todos.** Nada fora da allowlist de
  um recurso jamais foi alcançável, então nenhuma resposta ganha dados; um
  caminho profundo perde a cauda. Um código de status muda junto: um caminho
  cuja cauda excessivamente profunda nomeia um relacionamento que o recurso
  não permite é truncado antes de qualquer validação, então ele agora retorna
  `200` com os segmentos que sobreviveram, onde o caminho completo antes
  retornava `400` - ajuste qualquer cliente ou teste que faça assertion sobre
  essa rejeição. Suba o teto com `suprnova::max_relationship_depth(n)` se a
  sua API documenta caminhos mais longos que isso.
- **O `DatabaseConfig` ganhou cinco campos públicos.** Código que constrói um
  com um struct literal não compila mais. Use `DatabaseConfig::from_env()` ou
  `DatabaseConfig::builder()`, que preenchem os campos novos com os padrões
  que preservam o comportamento de pool de hoje.

## 1.3.3 - 2026-08-25

### Adicionado

- **Conexão de fila com failover.** O `FailoverQueueDriver` embrulha uma
  lista ordenada de conexões: um push que a primeira recusar é repetido na
  seguinte, e assim por diante, lista abaixo. Conecte-o pelo env com
  `QUEUE_DRIVER=failover` mais `QUEUE_FAILOVER_CONNECTIONS=redis,database`
  (cada entrada lê as variáveis do próprio driver, então uma entrada
  `database` ainda precisa de `DB::init()` antes e ainda traz o próprio
  armazenamento de jobs falhados), ou construa-o direto com
  `FailoverQueueDriver::new(vec![(label, driver), ...])`. Só as escritas
  caem pela lista: `push` e `bulk_push` percorrem a lista, enquanto `pop`,
  `pop_from`, `ack`, `nack`, `release`, `settle`, `clear`, os quatro
  contadores e as três listagens de inspeção delegam para a primeira
  conexão e para nenhuma outra, porque um token de reserva só faz sentido
  para o driver que o emitiu. A consequência operacional está documentada
  em vez de encoberta: um worker na conexão de failover drena só a
  primária, então tudo que fez failover para um fallback precisa do
  próprio worker. O `bulk_push` faz push de cada envelope separadamente em
  vez de encaminhar um batch, o que preserva o `available_at` de cada
  envelope (Laravel #60950) e impede que um batch que a primária aceitou
  pela metade seja empurrado inteiro para o fallback. Uma recusa despacha
  `queue::events::QueueFailedOver { connection, job_name, exception }`,
  disparado por borda: uma conexão se reporta uma vez quando entra em
  falha e fica quieta até um push posterior ter sucesso nela e rearmá-la,
  então uma indisponibilidade produz um alerta em vez de um por dispatch.
  Quando toda conexão recusa, o push retorna o erro da última conexão. Uma
  lista de conexões vazia, uma `QUEUE_FAILOVER_CONNECTIONS` ausente ou em
  branco, uma entrada `failover` aninhada e uma entrada que nomeie um
  driver inexistente são todos erros de boot - o comportamento de avisar e
  recair para memória fica no próprio `QUEUE_DRIVER`, onde um erro de
  digitação não consegue emendar um backend efêmero em uma cadeia durável.
- **API de inspeção de fila.** `Queue::pending_jobs(queue)` / `delayed_jobs`
  / `reserved_jobs` listam os envelopes de fato por trás dos contadores
  `pending_size`/`delayed_size`/`reserved_size` já existentes, como DTOs
  `InspectedJob` (`id`, `queue`, `name`, `attempts`, `payload`,
  `created_at`) - espelham o `InspectedJob` do Laravel. Um único filtro de
  fila `Option<&str>` colapsa o par `pendingJobs($queue)` /
  `allPendingJobs()` do Laravel (e os equivalentes de
  `delayedJobs`/`reservedJobs`) em uma chamada cada. O padrão da trait
  `QueueDriver` é um `Err` honesto - não o padrão de coleção vazia do
  Beanstalkd/SQS do Laravel, que se lê como "nada enfileirado" mesmo
  quando claramente há -, então um driver que não implementou a inspeção
  diz isso; `sync`/`null` sobrescrevem com `Ok(vec![])` porque para eles
  essa é mesmo a verdade. Os drivers de memória, de banco de dados e de
  Redis implementam todos a listagem completa: o armazenamento de jobs
  atrasados do driver de memória saiu de um `DelayQueue<Envelope>` puro
  (que não pode ser iterado) para um `DelayQueue<Uuid>` mais um mapa
  indexado por id; o driver de banco de dados reaproveita os predicados
  exatos dos contadores de tamanho mais `ORDER BY available_at`, e uma
  linha cujo `envelope_json` não decodifica ainda assim é listada (`id:
  None`, `payload: {"unparseable": true}`) em vez de descartada, para que
  uma linha envenenada não consiga cegar um operador para o resto da fila;
  o `reserved_jobs` do Redis é limitado às reservas em processo deste
  consumidor (documentado), e `pending_jobs` varre o stream via `XRANGE`
  em lotes. O `Queue::fake()` ganhou helpers `pending_jobs()`/`delayed_jobs()`
  correspondentes, projetando os pushes registrados com `attempts` sempre
  `0` e `created_at` sempre `None`.
- **Dispatch pós-commit.** O `Job::after_commit()` segura um push até a
  `DB::transaction` ao redor fazer commit, para que um worker em outro
  processo nunca consiga dar pop em um envelope que descreve linhas que a
  transação ainda não tornou duráveis. O push inteiro espera, não só a
  escrita do driver: a montagem do envelope, `JobQueueing` e `JobQueued`
  acontecem todos no momento do commit, então nenhum listener chega a ser
  avisado de um job que um rollback depois descarta. Um rollback descarta
  o push por completo; fora de uma transação o push acontece na hora, que
  é o que permite a um tipo de job declarar a adesão sem que todo ponto de
  dispatch saiba se o seu caminho de código é transacional. Por dispatch,
  o `EnvelopeOverrides::after_commit` supera o job: `Some(true)` (com o
  atalho `Queue::push_after_commit(job)`) adia um job que não aderiu, e
  `Some(false)` é o `beforeCommit()` do Laravel. Um `Queue::push` adiado
  resolve de novo o `Job::delay()` contra o commit em vez de contra o
  push, enquanto `Queue::push_later` / `later` / `later_with` carregam o
  timestamp absoluto do chamador sem alteração. O `Queue::push_unique`
  toma o seu lock de deduplicação imediatamente mesmo quando o envelope é
  adiado, então uma duplicata dentro da mesma transação continua sendo
  suprimida, e um rollback libera esse lock com escopo por dono. O
  `Queue::bulk` adia como uma unidade. O `Queue::fake()` registra um push
  na hora, adiamento e tudo, casando com o `Bus::fake` do Laravel. Um
  `DB::begin_transaction` manual nunca adia - ele não instala transação
  ambiente alguma, então não há commit em que pendurar um callback. Todo
  desfecho que deixa o commit sem acontecer compensa de forma idêntica,
  incluindo um `COMMIT` que o banco recusa e um `TxHandle` vazado que
  impede um commit, e o `Transaction::rollback_to` conta como um desses
  para o escopo que ele desfaz: um push adiado dentro de um savepoint é
  descartado quando aquele savepoint sofre rollback e o seu lock é
  liberado naquele instante, enquanto qualquer coisa registrada antes do
  savepoint fica intocada. Mail, notificações, batches e chains
  enfileirados ainda não adiam.
- **Jobs únicos até o processamento.** O `Job::unique_until_processing()`
  libera o lock de unicidade quando o processamento começa - depois da
  passagem do middleware do job, imediatamente antes de o handler rodar -
  em vez de segurá-lo pela janela `unique_for` inteira, que é o que você
  quer quando o lock existe para unificar duplicatas enfileiradas e não
  para serializar a execução. Um job que um middleware devolve para a fila
  mantém o seu lock, porque não começou a ser processado; um job que um
  middleware apaga ou manda para dead-letter abre mão do seu lock. A
  liberação tem escopo por dono: o `Queue::push_unique` registra o token
  de dono do lock de cache no envelope (`Envelope::unique_lock_owner`, um
  campo aditivo que deixa o formato do wire congelado idêntico byte a byte
  para todo push não único), e o worker libera com esse token, então uma
  tentativa reentregue nunca consegue forçar a liberação de um lock que um
  dispatch mais novo agora detém. A superfície de idempotência que dá
  suporte a isso também é pública: o `Idempotency::commit_on_success_owned`
  entrega o dono do lock ao corpo e o devolve, e o
  `Idempotency::release_owned(key, owner)` libera com escopo por dono,
  reportando `Ok(false)` em vez de um erro quando o lock está ausente ou
  pertence a outro dono. Jobs com `unique_id` simples ficam inalterados
  e continuam deixando o TTL de `unique_for` ser a janela de deduplicação.
- **O `Gate::default_denial_response` personaliza o formato padrão de uma
  negação nua.** Espelha o `Gate::defaultDenialResponse($response)` do
  Laravel. Definido uma vez - normalmente em `bootstrap::register()` -,
  ele remodela exatamente dois desfechos: um `false` nu (um gate
  booleano - `Gate::define` / `Gate::define_async`, incluindo um método
  `#[policy]` que retorna `bool` - ou um hook `before`/`after` que decidiu
  `false`) e uma avaliação que mais nada decidiu (uma habilidade
  indefinida sem opinião de hook também). Tudo isso costumava colapsar
  para um `Response::deny()` nu (um 403); agora aparece como o
  `Response` que o padrão carrega, por exemplo
  `Response::deny_as_not_found()` para um 404 que esconde a existência de
  um recurso em toda a aplicação em vez de gate a gate. O padrão se aplica
  somente ao `false` nu - um gate registrado com `define_with` /
  `define_async_with` já retornava o `Response` que queria, e esse sempre
  passa pelo `Gate::inspect` intocado, casando com a regra do próprio
  Laravel de que o padrão nunca substitui um objeto `Response` retornado.
  Um padrão no formato `Response::allow()` é rejeitado (registrado em log,
  ignorado) em vez de inverter silenciosamente todo gate booleano para
  permitido - veja o comentário de documentação de
  `Gate::default_denial_response` para o único ponto em que isto diverge
  de propósito do Laravel, que não tem salvaguarda desse tipo.
- **A família de regras de validação `Password` está disponível, incluindo
  a verificação `uncompromised()` do Have I Been Pwned.** O
  `Password::min(n)` mais os construtores de força (`.max()`,
  `.letters()`, `.mixed_case()`, `.numbers()`, `.symbols()`) portam
  literalmente as regexes da regra `Password` do Laravel - um espaço
  comum satisfaz `.symbols()`, casando com a classe de separadores `\p{Z}`
  do Laravel. O `.uncompromised()` (ou o
  `.uncompromised_with_threshold(n)`) confere a senha contra a API de
  intervalos com k-anonimato do Have I Been Pwned: só os 5 primeiros
  caracteres do hash SHA-1 da senha chegam a sair do processo, e uma falha
  de rede, um timeout ou uma resposta não 2xx falham aberto em vez de
  bloquear cadastros, exatamente como o `NotPwnedVerifier` do Laravel.
  Como essa verificação é uma ida e volta HTTP, o `Password` é a única
  regra embutida que implementa tanto `Rule` (só força, para linhas
  síncronas de `validate!`) quanto `AsyncRule` (força e depois a
  verificação HIBP, para `after_validation_async`) - chamar o caminho
  síncrono em um `Password` configurado com `uncompromised()` é um erro
  explícito voltado ao desenvolvedor, e não uma omissão silenciosa. O
  `Password::defaults_with(...)` define o padrão global ao processo que o
  `Password::defaults()` devolve. Nova variável de ambiente
  `HIBP_TIMEOUT_SECS` (padrão 30s). O `Http::fake_response_text(...)` é o
  novo irmão de corpo bruto do `fake_response(...)` para testes contra
  APIs upstream de `text/plain` como a do HIBP.
- **Uma tarefa agendada agora pode nomear o fuso horário em que a sua
  expressão cron é lida, e o `schedule:list` consegue renderizar o
  agendamento inteiro em qualquer fuso.** O `.timezone(chrono_tz::Tz)`
  fixa uma tarefa, o `.try_timezone("Area/City")` é o irmão falível para
  um nome de fuso que só existe em runtime, e o `Schedule::timezone(tz)`
  define um padrão para toda tarefa registrada depois dele. Nada muda para
  uma tarefa que não fixa fuso: ela continua sendo avaliada contra o fuso
  local do processo. Um fuso fixado afeta somente se a tarefa está devida -
  o agendador ainda dá um tick por minuto de processo e o gate de
  deduplicação do mesmo minuto fica intocado. Note que um fuso que observa
  horário de verão faz alguns minutos de relógio de parede acontecerem
  duas vezes e outros não acontecerem de jeito nenhum, então uma tarefa
  fixada em um minuto desses pode rodar duas vezes ou ser pulada; o
  capítulo de agendamento traz o aviso completo. O `schedule:list` ganhou
  uma opção `--timezone` e duas colunas: o fuso em que uma expressão
  impressa está escrita e o próximo minuto em que a tarefa dispara. A
  expressão de uma tarefa fixada é reescrita para o fuso da listagem,
  quebrando em várias linhas quando atravessa a meia-noite ali, e é
  deixada exatamente como escrita quando uma reescrita fiel é impossível -
  atravessando uma transição de horário de verão, quando uma virada de dia
  teria de mover juntos um dia do mês e um dia da semana restritos, ou
  quando teria de decidir quantos dias tem fevereiro. O `chrono_tz::Tz` é
  reexportado da raiz do crate, então os apps consumidores não adicionam
  `chrono-tz` ao próprio `Cargo.toml`.
- **Um subsistema de imagens no formato do Laravel, em `suprnova::media`,
  atrás da feature `media` ligada por padrão.**
  `Image::from_bytes/from_path/from_disk/from_upload/from_stream` monta um
  pipeline lazy - `resize`, `scale`, `crop`, `cover`, `contain`, `rotate`
  em qualquer ângulo, `flip_vertically`/`flip_horizontally`, `blur`,
  `sharpen`, `grayscale`, `to_format`, `quality` - encerrado com
  `to_bytes`, `to_response`, `save`, `store`, `dimensions`, `mime_type` ou
  `dominant_color`. Lê e escreve PNG, JPEG, WebP, GIF e BMP; a saída AVIF
  fica adiada até o encoder AV1 interno ser publicado, momento em que ela
  será uma nova variante de `OutputFormat` e nenhuma outra mudança. Como
  na divisão `gd`/`imagick` do Laravel, há dois drivers:
  `IMAGE_DRIVER=oxideav` (o padrão) roda sobre a família de codecs em Rust
  puro [OxideAV](https://github.com/OxideAV), sem biblioteca nativa e sem
  nada a instalar, e `IMAGE_DRIVER=magick` chama um ImageMagick 7
  instalado no host para um suporte mais amplo de entrada, HEIC incluído.
  Os limites de decodificação (`IMAGE_MAX_DIMENSION`,
  `IMAGE_MAX_ALLOC_BYTES`) são conferidos contra o cabeçalho da própria
  entrada antes de qualquer coisa ser alocada - incluindo o bitstream
  interno de um WebP estendido, cujo tamanho de canvas consultivo não pode
  ser usado para contrabandear um frame maior pelo gate - e todo o
  trabalho de pixel roda em uma thread bloqueante. O driver `magick` fixa
  o coder de entrada pelo nome em vez de deixar o ImageMagick escolher um
  a partir dos bytes, e limita toda invocação com
  `IMAGE_MAGICK_TIMEOUT_SECS`. O `ImageDriver` é a fronteira de trait para
  qualquer outra coisa. O módulo se chama `media` porque as superfícies de
  áudio e vídeo apoiadas no OxideAV vão viver ao lado dele.
  [Imagens](images.md)
- **O gate de WebP carrega um limite fixo e não configurável.** Um WebP
  declara o seu tamanho decodificado real no chunk mais interno do
  bitstream, então o framework percorre o container para encontrá-lo; essa
  varredura visita no máximo 4096 chunks por nível e segue dois níveis de
  aninhamento, e um arquivo além de qualquer um dos dois é recusado em vez
  de medido. Informar um número vindo de uma varredura inacabada seria um
  gate que chunks de enchimento em quantidade suficiente conseguiriam
  contornar. Nenhuma variável `IMAGE_MAX_*` o afeta, e o erro diz
  exatamente isso. Uma animação de 300 quadros não é afetada; uma de 4100
  quadros é recusada. [Imagens](images.md#one-bound-is-not-configurable)

- **Agora o OAuth pode ser instalado sem substituir a autoridade de senha e
  de sessão que a aplicação já tem.** O `MagnetarOAuthOnlyConfig` e o
  `init_magnetar_oauth_only` instalam a cerimônia padrão e o mecanismo de
  provedor, deixando vazios os slots de senha e de passkey. Aplicações com
  uma tabela `users` existente podem chamar o `verify_oauth_identity`,
  mapear elas mesmas o subject verificado do provedor e estabelecer a sessão
  normal do framework.

### Alterado

- **A `DB::transaction` agora pode retornar `Err` depois de um commit
  bem-sucedido**, quando um callback pós-commit falha: a mensagem diz
  `after-commit callback failed (the transaction itself committed): …`, o
  valor de retorno da closure se perde e as escritas dela não. A
  `DB::transaction_with_attempts` nunca repete esse erro, por mais que a
  mensagem do próprio callback tenha cara de deadlock - reexecutar uma
  closure cujas escritas já são duráveis as aplicaria duas vezes.
- **Nova chave do catálogo de validação:
  `validation-password-unverifiable`.** Um `UncompromisedVerifier`
  personalizado que retorna `Err` não coloca mais o seu próprio texto de
  erro literalmente no corpo do 422. Esse texto passa a ser registrado em
  `error`, e a resposta carrega esta chave, que renderiza como "The
  { $field } could not be checked against known data leaks. Please try
  again." - a verificação não rodou, o que não é o mesmo que a senha ser
  ruim, e detalhe de infraestrutura não pertence a uma resposta de
  cliente. Um app que traz o próprio catálogo de validação precisa
  acrescentar a chave, ou os seus usuários vão ver o fallback embutido em
  inglês.
- **O validador de upload `Image` agora é `ImageFile`.** O
  `suprnova::Image` é o novo tipo de pipeline de manipulação de imagens,
  casando com `Illuminate\Image\Image`, e a regra de upload por bytes
  mágicos assume o nome que o Laravel dá à mesma classe de regra,
  `Illuminate\Validation\Rules\ImageFile`. A migração é uma linha por
  ponto de uso: `UploadedFile<(Image, MaxSize<N>)>` vira
  `UploadedFile<(ImageFile, MaxSize<N>)>`. Churn pré-1.0 absorvido pelo
  modelo de distribuição por tag git.

### Removido

- **A dependência direta e não usada `image` se foi.** Ela era uma
  dependência base com zero pontos de uso em qualquer lugar do workspace,
  puxando codecs de JPEG, PNG, WebP e GIF à toa; removê-la tira `gif`,
  `image-webp`, `zune-jpeg`, `color_quant` e `weezl` da árvore. O crate em
  si ainda aparece transitivamente, só com a feature `png`, por trás da
  renderização de QR code do `totp-rs`. O novo subsistema de imagens é
  construído sobre os crates do OxideAV, atrás da feature `media`.

### Corrigido

- **Instalar o OAuth não força mais as aplicações apoiadas em provedor à
  validação de vinculação web do Magnetar.** O caminho completo do
  `init_magnetar` continua atômico e inalterado. O caminho somente de OAuth
  reserva os slots de mecanismo durante a construção, publica somente OAuth
  e falha em vez de misturar duas autoridades de autenticação.

### Atualizando

- **`Image` agora é um tipo diferente; o validador de upload é
  `ImageFile`.** Quebra o código-fonte de quem usa a regra de upload por
  bytes mágicos. Renomeie em todo ponto de uso:
  `UploadedFile<(Image, MaxSize<N>)>` vira
  `UploadedFile<(ImageFile, MaxSize<N>)>`. O `suprnova::Image` continua
  resolvendo, mas agora é o tipo de pipeline de manipulação de imagens,
  então um rename esquecido falha na compilação em vez de mudar o
  comportamento em silêncio.
- **O `EnvelopeOverrides` ganhou um campo público
  `after_commit: Option<bool>`.** Toda construção neste repositório e nos
  templates com scaffold usa `..Default::default()`, que não precisa de
  mudança. Código que monta um `EnvelopeOverrides` com um literal de
  struct exaustivo tem de nomear o campo novo; `after_commit: None`
  mantém o comportamento de hoje, que é deferir ao `Job::after_commit()`.
  Nada mais muda: o `after_commit()` tem `false` como padrão, então nenhum
  job existente passa a esperar por um commit pelo qual não esperava
  antes.
- **O `Envelope` ganhou um campo público
  `unique_lock_owner: Option<String>`.** O formato do wire está
  inalterado - o campo é `#[serde(default)]` e é pulado quando é `None`,
  então os envelopes fazem ida e volta idênticos byte a byte nas duas
  direções e o `schema_version` continua em 2 -, mas qualquer código que
  monte um `Envelope` com um literal de struct agora tem de nomeá-lo.
  Acrescente `unique_lock_owner: None`, a menos que você esteja
  deliberadamente carregando um lock de unicidade através do push. Código
  que só lê envelopes, ou que os monta pelo `Queue::push` e seus irmãos,
  não precisa de mudança.

- Use o `init_magnetar_oauth_only` em vez do `init_magnetar` quando a
  aplicação já for dona dos usuários, das senhas, das sessões do framework e
  do estado de lembrar-me. Callbacks somente de OAuth usam o
  `verify_oauth_identity`; aplicações Magnetar completas continuam usando o
  `complete`.

## 1.3.2 - 2026-08-25

### Adicionado

- **Providers de OAuth agora podem ser registrados através do
  `MagnetarConfig::oauth`.** O Suprnova reexporta o contrato
  `OAuthProvider`, todos os cinco tipos de provider e de configuração de
  primeira parte, e os tipos de HTTP, de revogação, de limitador de abuso,
  de autorização e de vínculo automático de que uma aplicação precisa.
  Providers customizados não exigem mais uma dependência direta de
  `suprnova-magnetar` nem um `MagnetarHostEngine` mantido à mão.

- **Um transporte de OAuth de produção e um adaptador de limitador do
  framework agora vêm na raiz do crate.** O `ReqwestOAuthTransport`
  implementa a E/S de token, de userinfo e de revogação com
  redirecionamentos desabilitados por padrão, um timeout de 30 segundos, um
  `User-Agent` padrão e um teto de 1 MiB para a resposta. O
  `FrameworkAbuseLimiter` reutiliza o `RateLimiterDriver` configurado; as
  aplicações não escrevem mais nenhum dos dois adaptadores à mão.

### Corrigido

- **O `init_magnetar` agora publica o OAuth junto com os serviços de senha e
  de passkey como uma única instalação reservada.** O serviço de OAuth é
  construído antes da publicação, e os três slots de engine ficam ocultos
  enquanto a reserva estiver ativa. Uma configuração de OAuth que falhe ou
  esteja duplicada não consegue deixar o estado de senha e de passkey
  visível sem o registro de OAuth configurado.

- **Providers customizados podem fornecer headers de userinfo.** O
  `OAuthProvider::userinfo_headers` é mesclado com o header bearer de posse
  do host, o que atende a requisitos como o `User-Agent` do GitHub e headers
  `Accept` de tipo de mídia sem permitir que um provider substitua o
  `Authorization`.

### Atualizando

- **A migração para o Magnetar em `4faaa933` removeu o caminho de instalação
  de OAuth do Torii sem ligar o substituto dele ao inicializador padrão.** O
  workaround antigo exigia construir um host engine customizado, chamar o
  `oauth_service` e instalar o adaptador em separado. Troque esse workaround
  por `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` e uma
  única chamada de `init_magnetar`.

- **Providers da comunidade para o GitHub precisam tratar o e-mail
  verificado de forma explícita.** O `/user` do GitHub costuma omitir o
  e-mail que não é público, enquanto o endereço primário verificado exige o
  `/user/emails`. Devolva `email: None` para usar a cerimônia de completar o
  e-mail, ou aponte o `userinfo_endpoint` para um adaptador de host que
  combine as duas respostas; nunca trate um endereço público mas não
  verificado como prova de posse.

## 1.3.1 - 2026-08-24

### Corrigido

- **Aplicações respaldadas por provider voltam a conseguir redefinir a senha
  de usuários verificados.** Quando nenhum engine do Magnetar está
  instalado, o `PasswordReset` usa um `UserProvider` explicitamente capaz de
  redefinir e os `auth_flow_tokens` do framework para contas já verificadas.
  O `EloquentUserProvider<M>` adere quando `M` implementa
  `MustVerifyEmail + CanResetPassword`; nenhuma migração de `app_users` é
  necessária.
- **A linha de framework publicada agora contém os dois conjuntos de reparo
  pós-release.** O layout e os cabeçalhos do changelog 1.3.0 traduzido, a
  quebra de linha em CJK, as âncoras localizadas, os termos de glossário e a
  pontuação da prosa são reconciliados em vez de ficarem divididos entre
  branches local e remota divergentes.
- **O endurecimento pós-tag da CLI e do Magnetar está incluído.** A limpeza
  do processo de desenvolvimento usa o fallback de grupo de processos
  concluído, e os contratos de qualificação locais cobrem as refs lançadas e
  as trilhas de SQLite do SDK de plugins.

### Segurança

- **O fallback por provider nunca trata a redefinição de senha como primeira
  prova de posse da caixa de e-mail.** Endereços desconhecidos e não
  verificados recebem a mesma resposta de "nenhum e-mail enviado". Instale o
  Magnetar quando uma conta não verificada tiver de provar a posse da caixa
  de e-mail através da redefinição, para que a limpeza de credenciais, o
  avanço da época de autenticação e a revogação continuem atômicos. A
  conclusão pelo fallback por provider reporta falhas de revogação de sessão
  e de remember do framework através do `PasswordResetOutcome`.

### Atualizando

- **Mova toda dependência Git de `v1.3.0` para `v1.3.1`.** Aplicações com a
  própria tabela `users` mantêm o `UserProvider` que configuraram; elas não
  inicializam o engine `app_users` padrão só para redefinir a senha de uma
  conta já verificada. Aplicações que usam credenciais do Magnetar ou a
  primeira prova de uma conta não verificada continuam inicializando o
  Magnetar.

## 1.3.0 - 2026-08-24

### Segurança

- **O Magnetar agora restringe mutações de credencial e de sessão ao ator
  autenticado e à época de autenticação da conta.** Escritas de senha, de
  passkey, de conta vinculada, de dois fatores, de sessão opaca, de JWT, de
  remember, de OAuth e de autorização de dispositivo recusam atores
  obsoletos ou revogados. A primeira prova bem-sucedida de redefinição de
  senha, de magic link ou de e-mail verificado por OAuth em uma conta não
  verificada avança a época e remove atomicamente credenciais provisórias,
  sessões, estado de remember e cadastro de TOTP feito por um invasor.
  Contas verificadas preservam credenciais legítimas durante a redefinição
  de senha. A verificação de e-mail exige o dono autenticado do token, e o
  OAuth nunca vincula automaticamente uma conta existente não verificada
  apenas pelo e-mail.

- **Um `_previous.url` relativo ao protocolo não consegue mais produzir um
  open redirect para fora da origem através do `Redirect::back()`, nem no
  lado da escrita nem no lado da leitura.** O `SessionMiddleware` não
  persiste mais uma URL atual relativa ao protocolo: a escrita passa pelo
  mesmo sanitizador que o `InertiaValidationRedirectMiddleware` usa na
  verificação de `Referer`, e um caminho de solicitação no formato `//host`
  (ou que carregue um byte de controle ASCII) nunca é registrado - sem isso,
  a rota `fallback!` de uma aplicação (o padrão clássico de app shell do
  Inertia/SPA, em que qualquer caminho sem correspondência responde `200`)
  poderia fazer `GET //evil.test/anything` persistir aquele caminho
  literalmente. O `SessionData::previous_url()` agora aplica a mesma
  verificação em toda **leitura** também, então um cookie de sessão que
  sobreviveu a uma atualização vinda de um release anterior a esta correção -
  já carregando um valor cru, não sanitizado, que nenhuma escrita do
  processo atual jamais produziu - se autocorrige para "nada registrado" em
  vez de ser tratado como confiável. Juntos, nem um cookie envenenado antigo
  nem uma solicitação maliciosa nova conseguem entregar ao
  `Redirect::back()`, ao `Redirect::refresh()` ou ao `url::previous()` um
  `Location` fora da origem. Quando um valor falha em qualquer uma das duas
  verificações, ele é tratado como ausente em vez de ser substituído por um
  valor sintetizado, então uma URL anterior genuinamente boa nunca é
  sobrescrita.
- **A verificação de `Referer` da ponte de redirecionamento de validação do
  Inertia fechou mais dois desvios de mesma origem.** O destino `303` do
  `InertiaValidationRedirectMiddleware` só recusava um `Referer` que
  começasse com o prefixo literal `//` ou `/\` - um valor como
  `Referer: /<TAB>/evil.test` passava, porque o parser de URL da WHATWG
  remove tab e quebra de linha ASCII da string inteira antes de comparar
  origens, então um navegador lê aquilo como `//evil.test` e segue o `303`
  para fora da origem. A verificação agora recusa qualquer byte de controle
  ASCII (C0 ou DEL) em qualquer posição do candidato, e não apenas dentro
  dos dois prefixos nomeados. Em separado, o fallback de último recurso - o
  próprio caminho da solicitação que falhou, usado quando nem o `Referer`
  nem a URL anterior da sessão servem - nunca era sanitizado: um
  request-target HTTP em origin-form pode sintaticamente começar com `//`,
  então um cliente cru ou um proxy que não normaliza também podia
  transformar o "último recurso seguro" em um redirecionamento para fora da
  origem. As duas pernas agora compartilham uma única verificação de caminho
  relativo à raiz, recaindo para `/` se até o próprio caminho da solicitação
  falhar nela.
- **O texto cifrado de um cookie agora é vinculado ao seu nome lógico de
  cookie com AAD v2 contextualizado.** O `Cookie::encrypted` /
  `Cookie::read_encrypted_for` impedem que um valor cunhado para um slot de
  cookie seja decifrado em outro slot, enquanto a vinculação ao nome lógico
  mantém segura uma troca posterior do prefixo de rede `__Host-` /
  `__Secure-`. A janela de compatibilidade sem versão tenta v2 em todo o
  chaveiro e depois v1 em todo o chaveiro, então cookies existentes
  sobrevivem ao rollout; o fallback de v1 preserva a antiga fraqueza de
  replay até sua remoção agendada para a 1.4.0.
- **Os prefixos dos cookies de sessão e de remember-me são validados no boot
  e aplicados na hora da renderização.** O `SESSION_COOKIE_PREFIX=__Host-`
  exige `Secure`, `Path=/` e nenhum `Domain`; o `__Secure-` exige `Secure`.
  Combinações inválidas no boot falham antes de servir, e o renderizador
  reescreve headers com prefixo inválido em vez de deixar que os navegadores
  os descartem em silêncio.

### Adicionado

- **A autenticação do Suprnova agora roda sobre o engine interno do
  Magnetar.** A facade `Auth`, de propriedade do framework, preserva os
  pontos de chamada existentes de senha, magic link, passkey, OAuth, bearer,
  lockout, sessão e dois fatores, ao mesmo tempo que remove a dependência do
  Torii. O engine padrão instala os adaptadores de senha/sessão e de passkey
  atomicamente, guarda as leases de entrega do ciclo de vida no banco de
  dados da aplicação e compartilha as identidades `i64` canônicas de
  `app_users` da aplicação.
- **Um runner de migração de autenticação ciente de formato agora cobre
  origens Torii, Suprnova web e Suprnova API.** Execuções de dry run
  vinculam um id de plano estável a fingerprints duráveis de linha e de
  schema, mais as decisões de identidade no destino. O passo de apply usa
  importações transacionais, ledgers de retry, limpeza de posse do formato e
  recusa em caso de colisão. O MySQL usa uma troca por shadow protegida por
  barreira de escrita, com journals de pré-cópia, paridade de linhas e de
  schema, renomeações retomáveis e uma restauração que preserva a limpeza.
- **`MAIL_DRIVER=file` grava um `.eml` RFC 5322 por mensagem** em
  `MAIL_FILE_PATH` (padrão `storage_path("mail")`; um valor relativo se
  ancora no diretório base da aplicação, não no CWD do processo), então o
  e-mail local pode ser aberto em um cliente de e-mail em vez de lido em uma
  linha de log. O arquivo carrega o mesmo superconjunto de headers que o
  SMTP emite, incluindo `X-Priority`, `Importance`, `X-Tag`, `X-Metadata-*`
  e `Return-Path`. Assim como `log` e `memory`, ele não entrega: um boot em
  produção o recusa a menos que
  `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **O `FrameworkError::External` carrega o erro que ele envolve.** O
  `FrameworkError::from_external(e)` e o
  `FrameworkError::from_external_with("saving user", e)` mantêm o erro
  original alcançável como uma origem `std::error::Error` em vez de
  derretê-lo em uma string. O `FrameworkError::external_source()` o devolve
  para downcasting - use esse método em vez de `source()`, que entrega o
  handle `Arc` compartilhado. Os dois construtores mapeiam para HTTP 500.
- **Logs de 5xx agora renderizam a cadeia completa de origens do erro.** O
  `render_error_chain` percorre `source()` e está ligado à linha de log de
  erro do framework, ao payload do evento `ErrorOccurred` e ao campo
  `debug_message` emitido sob `APP_DEBUG=true`. Os corpos de resposta
  voltados ao cliente não mudam, e os corpos de 5xx continuam sanitizados.
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` /
  `try_scroll_wrapped`.** Aninhe a instrução de merge de uma prop de scroll
  sob `<key>.<wrap_key>` em vez da chave nua - `mergeProps: ["users.data"]`
  em vez de `["users"]` - para um valor que é ele próprio um envelope
  (`{ data: [...], meta: {...} }`). O `ScrollProp` do Laravel embrulha sob
  `"data"` incondicionalmente; os paginadores nativos do Suprnova devolvem
  um array de linhas nu, então isso é opt-in em vez de um padrão que todo
  chamador tem de contornar. A nova trait `ProvidesScrollMetadata`
  (`page_name` / `previous_page` / `next_page` / `current_page`, com um
  `scroll_metadata()` padrão) espelha a interface de mesmo nome do Laravel
  para um paginador que este crate não conhece; `LengthAwarePaginator`,
  `Paginator` e `CursorPaginator` agora a implementam em vez de montar
  `ScrollMetadata` à mão. Os campos de `.match_on(...)` de uma prop de
  scroll agora também são emitidos em `matchPropsOn`, acompanhando o
  `resolveMergeMatchingKeys` do Laravel (`Response.php:641-652`), que dobra
  o `matchesOn()` de um `ScrollProp` como o de qualquer outra prop de merge -
  a entrada de match se chaveia por onde a prop de fato faz o merge,
  `<key>` desembrulhada ou `<key>.<wrap_key>` sob `.scroll_wrap(...)`.
- **`Prop::merge_with_path`, `match_on` com vários campos e props de merge
  respaldadas por resolver.** O `Prop::merge_with_path(path)` faz merge de
  um campo aninhado dentro do valor de uma prop em vez da prop inteira - o
  `Prop::eager(v).merge().merge_with_path("data")` emite
  `mergeProps: ["<key>.data"]`, e uma prop que faz merge por caminho nunca
  faz merge também da própria raiz; o `.deep_merge()` ignora esse caminho,
  já que um deep merge de qualquer forma já recursa em cada campo. O
  `Prop::match_on` agora aceita um campo ou vários em uma só chamada
  (`match_on(["id", "slug"])`), além do encadeamento
  `match_on("id").match_on("slug")` que a composição de `Prop` já suporta. O
  `InertiaResponse::merge_lazy` / `merge_lazy_with` acrescentam os irmãos
  respaldados por resolver de `.merge` / `.merge_with`, acompanhando o
  `Inertia::merge(fn () => ...)` do Laravel.
- **O `only`/`except` de reload parcial entende notação de ponto.** O
  `X-Inertia-Partial-Data: user.name` estreita a prop `user` para
  `{ name: ... }` em vez de exigir o valor inteiro ou nada; o
  `X-Inertia-Partial-Except: user.email` poda apenas aquele campo, deixando
  o resto de `user` no lugar. O `except` vence em um caminho que os dois
  headers nomeiam, uma entrada nua continua significando a prop inteira, e
  um caminho aninhado desconhecido ou de tipo incompatível é descartado em
  silêncio sem tocar nos irmãos. Props `Always` não são afetadas - elas
  sempre viajam inteiras.
- **Aninhamento de props por chave com ponto.** O
  `.with("user.name", value)` (e qualquer outro método que anexa prop, eager
  ou resolvido) agora aninha em `props.user` em vez de enviar uma chave
  literal `"user.name"`, acompanhando o desempacotamento de
  `resolveArrayableProperties` do Laravel, baseado em `Arr::set`. Duas
  chamadas que compartilham um prefixo - `.with("user.name", …)` e depois
  `.with("user.age", …)` - se acumulam em um único objeto; uma chave sem
  ponto não é afetada. As chaves do registro compartilhado de
  `App::inertia_share*` se aninham do mesmo jeito na rede. O
  desempacotamento só toca *chaves* de prop de nível superior - ele nunca
  recursa no valor de uma prop, então um conjunto `errors` de validação
  mantém quaisquer nomes de campo com ponto que carregue internamente.
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`.** O
  `Inertia::getShared` / `Inertia::flushShared` do Laravel, lendo e limpando
  o registro estático de compartilhamento (`App::inertia_share` / `_lazy` /
  `_once`). O `inertia_shared` suporta a mesma notação de ponto que o
  `inertia_share` no lado da leitura; ele devolve `None` para um
  compartilhamento lazy ou once (não há solicitação contra a qual
  resolvê-lo) e para uma chave não registrada. O `flush_inertia_shared`
  limpa apenas o registro estático - um provider de trait registrado via
  `App::register_inertia_shared` fica intacto, como no Laravel (não há
  estado por solicitação ali para limpar).
- **`InertiaResponse::always_with(key, resolver)`.** O irmão com resolver
  assíncrono de `.always(key, value)`, para uma prop sempre incluída e cara
  o bastante para valer a pena resolver de forma lazy - o
  `Inertia::always(fn () => …)` do Laravel (o `AlwaysProp` aceita qualquer
  valor, closures inclusive).
- **O `InertiaSharedData::share` agora recebe o nome do componente da
  página**, então um provider pode variar sua saída por página - o
  `RenderContext` do Laravel. Veja Atualizando.
- **Composição de props do Inertia.** Uma `Prop` agora carrega flags
  ortogonais em vez de ser uma entre nove variantes fechadas, então uma
  única prop pode ser deferred *e* mergeável, mergeável *e* cacheada, ou
  opcional *e* cacheada - as combinações que o protocolo do Inertia 3 espera
  e que um enum fechado não conseguia soletrar. Construa uma com
  `Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent`,
  encadeie `.always()`, `.optional()`, `.defer()`, `.group()`, `.rescue()`,
  `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on()`, `.once()`,
  `.as_key()`, `.until()`, `.fresh()`, `.scroll()` e anexe-a com o novo
  `InertiaResponse::prop(key, prop)`. Uma prop `defer().merge()` é anunciada
  sob `deferredProps` na primeira renderização e chega sob `mergeProps` na
  solicitação seguinte. Os novos tipos `MergeMode` e `Visibility` descrevem
  as flags; todo atalho de builder existente (`.with`, `.always`, `.lazy`,
  `.optional`, `.defer`, `.merge*`, `.once*`) permanece inalterado.
- **Pausa / retomada de fila.** `Queue::pause(connection, queue)` / `resume`
  / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` /
  `paused_queues(connection, &queues)`, respaldados pelo `Cache` do mesmo
  jeito que o sinal de restart - o `resume_all` não limpa uma pausa por
  fila, acompanhando o Laravel. O portão de reivindicação do worker fica
  logo antes de cada pop, então um job em voo sempre termina; uma pausa
  global curto-circuita a filtragem `--queue=...` do mesmo jeito que o
  `pausedQueues` do Laravel, e uma pausa por fila só tem efeito em um worker
  iniciado com uma lista `--queue=...` explícita. Novos comandos de CLI
  `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]` (alias
  `queue:continue`), mais `QUEUE_PAUSABLE=false` para um operador
  desabilitar o recurso - um worker não pausável ignora sinais de pausa, e o
  próprio `queue:pause` se recusa a rodar. Novos eventos: `QueuePaused` /
  `QueueResumed` / `QueuesPaused` / `QueuesResumed`.
- **`suprnova::testing::TestResponse`** - um wrapper fluente, no formato do
  `TestResponse` do Laravel, sobre a tripla `(status, headers, body)` que
  todo harness de teste HTTP já produz: `assert_status`, `assert_ok`,
  `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`,
  `assert_see`, `assert_header`, `assert_cookie` e (dado
  `.with_session_store(...)`) `assert_session_has`. Toda asserção devolve
  `&Self` e entra em panic na falha, o mesmo contrato do `expect!`. Nada em
  como um teste dirige uma solicitação precisa mudar.
- **O `suprnova new` faz o scaffold de uma entrada de SSR.** Todo starter
  (Svelte, React, Vue) agora traz `frontend/src/ssr.{ts,tsx}` e um script
  npm `build:ssr` (`vite build --ssr`), ligado ao seu próprio diretório de
  saída (`frontend/bootstrap/ssr/`), de modo que o bundle de SSR nunca
  colide com o build de cliente em `public/assets/`.
- **`InertiaConfig::ssr_bundle_path(path)` /
  `.ssr_ensure_bundle_exists(bool)`.** O gateway de SSR agora pode checar se
  o bundle construído existe em disco antes de despachar uma renderização,
  espelhando a config `ensure_bundle_exists` do Laravel - um worker que
  nunca foi iniciado, ou um bundle que nunca foi construído, falha rápido em
  vez de pagar o `ssr_timeout` em uma conexão que nunca ia dar certo. Opte
  por ele com `.ssr_bundle_path(...)`; ao contrário do `BundleDetector` do
  Laravel, o caminho nunca é detectado automaticamente, então configs de SSR
  existentes (e testes) que não definem um não são afetadas.
- **Falhas de validação em uma visita Inertia agora redirecionam de volta em
  vez de devolver `422` em JSON.** O `Inertia::install` registra um quarto
  middleware, o `InertiaValidationRedirectMiddleware`, que transforma um
  `422` de validação em uma solicitação `X-Inertia` em um `303` para a
  página do formulário com os erros no flash - então o `useForm().errors` se
  preenche sem nenhum código de handler. O cliente Inertia trata qualquer
  resposta sem um header `X-Inertia` como não-Inertia e mostra seu modal de
  erro, então o antigo `422` nunca conseguia chegar ao `form.errors`.
  Solicitações não-Inertia mantêm o envelope `422`, as execuções de teste do
  Precognition ficam intactas, e o `X-Inertia-Error-Bag` delimita o conjunto
  que vai para o flash. O destino do redirecionamento é o `Referer` de mesma
  origem, depois a URL anterior da sessão, depois o próprio caminho da
  solicitação passado por aquele mesmo sanitizador, recaindo para `/` se até
  isso falhar - nunca confiado literalmente.
- **`InertiaConfig::with_all_errors(bool)`** - mantém todas as mensagens de
  validação por campo em vez de colapsar para a primeira. Espelha o
  `Inertia\Middleware::$withAllErrors` do Laravel.
- **`suprnova::testing::AssertableInertia`** - asserções fluentes, no
  formato do `AssertableInertia` do Laravel, sobre um objeto de página do
  Inertia, parseado a partir de uma resposta JSON `X-Inertia` ou do elemento
  `<script data-page="app">` embutido na casca HTML de uma navegação dura:
  `component`, `url`, `version`, `prop`, `has`, `missing`, `where_`,
  `count`, `has_flash`. Construa uma a partir de um `HttpResponse` com o
  `AssertableInertia::from_response`, ou a partir de um `TestResponse` com o
  novo `TestResponse::assert_inertia()`. O `reload_only`, o `reload_except`
  e o `load_deferred_props` reproduzem um reload parcial contra uma closure
  `with_reload(...)` fornecida pelo chamador - os testes HTTP do Suprnova
  atravessam um socket real, então não há um único cliente de teste em
  processo contra o qual fixar o código.
- **`Cookie::queue`/`queued`/`unqueue`/`expire`.** Um pote de cookies local
  à task - o `CookieJar` do Laravel - deixa qualquer código enfileirar um
  cookie para a próxima resposta de saída sem segurar um `HttpResponse` ao
  qual anexá-lo: um listener de evento, um serviço vinculado no contêiner,
  um middleware antes do handler. Respaldado pelo mesmo slot por solicitação
  que o `Auth::login_remember` já usa para levar o cookie de remember-me
  além da fronteira do handler; o `SessionMiddleware` o drena para a
  resposta ao lado do cookie de sessão. O
  `Cookie::expire(name, path, domain)` enfileira um cookie de exclusão
  construído com o `Cookie::forget_with`. Exige o `SessionMiddleware` na
  cadeia de middlewares da rota - fora dela, as quatro chamadas são um no-op
  silencioso, acompanhando o comportamento do `App::flash` fora de um escopo
  de flash.
- **`HttpResponse::event_stream(stream, end)` e
  `HttpResponse::stream_json(stream)`.** O `ResponseFactory::eventStream` /
  `streamJson` do Laravel, e exatamente os formatos de rede que o
  `useEventStream` / `useJsonStream` do `@laravel/stream-{react,vue,svelte}`
  esperam. O `event_stream` enquadra um `Stream<Item = sse::StreamedEvent>`
  como `event: update` por item, a menos que o item nomeie o próprio evento,
  codifica em JSON qualquer payload que não seja string e acrescenta um
  quadro terminal configurável (o `EndSignal::default()` é
  `data: </stream>`; o `EndSignal::None` o omite). O `stream_json` transmite
  qualquer `Stream<Item = impl Serialize>` como um único array JSON
  descarregado incrementalmente. Os dois são construídos sobre o pipeline de
  corpo `sse`/`stream_bytes` existente, então compartilham seu comportamento
  de cancelamento e de isolamento de panic com o resto do framework.
- **O `suprnova serve` respawna um processo de desenvolvimento que sofreu
  crash em vez de derrubar a sessão inteira.** Backoff exponencial entre
  tentativas - 200ms, dobrando a cada crash consecutivo, com teto de 5s,
  voltando ao piso assim que um processo fica de pé por 30s. O
  `--no-restart` opta por sair disso e restaura o comportamento anterior. O
  `--restart-tries <N>` (padrão `5`, acompanhando o `--restart-tries=5` do
  Laravel) desiste de tentar de novo um processo depois desse número de
  crashes consecutivos, em vez de tentar para sempre, imprimindo uma
  mensagem acionável e deixando os outros processos - e a própria sessão -
  rodando. O `--timestamps` prefixa cada linha encaminhada com `HH:MM:SS`.
  Um novo array `[[serve.process]]` no `Suprnova.toml` deixa um projeto
  declarar seus próprios processos de desenvolvimento - o
  `DevCommands::register` do Laravel - para rodar ao lado do backend e do
  frontend, cada um com seu próprio prefixo `[name]` e uma cor opcional; uma
  chave desconhecida ou um `name`/`command` em branco em uma entrada agora é
  um erro duro de parse, em vez de ser ignorado em silêncio ou virar uma
  falha opaca de spawn mais tarde. O `--json` emite, em vez disso, um objeto
  JSON por linha (NDJSON) no stdout - eventos de início de processo, de
  saída, de encerramento, de restart agendado, de restart bem-sucedido, de
  desistência, de tipos regenerados e de shutdown, incluindo os avisos de
  regeneração do próprio observador de arquivos e o aviso de shutdown do
  handler de `Ctrl+C`, ambos os quais agora também ficam fora do stdout sob
  `--json` - para scripting e pipelines de log; combiná-lo com
  `--timestamps` é inofensivo, mas redundante, já que todo evento já carrega
  o próprio timestamp.
- **`RequestBuilder::retry_when(predicate)`.** Um predicado consultado antes
  de cada retry que a política embutida (`.retry(...)` /
  `.retry_non_idempotent(...)`) faria de outro modo, recebendo um
  `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }`.
  Ele compõe com a política em vez de substituí-la: `false` veta um retry
  que a política teria feito; ele nunca consegue forçar um retry além de
  `max_attempts`, nem um que a política não tentaria de outro modo (um
  status 4xx, ou um método não idempotente sem `retry_non_idempotent`).
- **`#[model(touches = [...])]` agora de fato toca.** Depois que um filho é
  criado, salvo, atualizado ou excluído, cada dono `BelongsTo` nomeado na
  lista recebe um `UPDATE <owner> SET updated_at = ? WHERE <key> = ?`, no
  mesmo executor da escrita que o disparou - então dentro de um
  `DB::transaction` o toque entra naquela transação e faz rollback junto com
  ela. Um dono cujo model tem `timestamps = false` é pulado, não escrito e
  não é um erro (o Laravel 13.25 fechou a mesma lacuna). Donos alcançados
  por uma chave estrangeira `NULL`, e donos com soft delete, também são
  pulados. Uma entrada de `touches` que não nomeia uma relação `BelongsTo`
  declarada agora é um erro de compilação; donos polimórficos ainda não são
  suportados.
- **`without_touching_on::<M, _, _>(fut)`** - o
  `Model::withoutTouchingOn([M::class], $cb)` do Laravel. Suprime tanto o
  `m.touch()` quanto qualquer cascata de dono que mire em `M`, enquanto
  donos de outros tipos continuam sendo atualizados. Os escopos se aninham,
  e o `without_touching` existente agora suprime a cascata de dono além das
  chamadas diretas a `touch()`.
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - o
  `touchOwners()` do Laravel, para quando você escreveu a linha do filho por
  um caminho que o framework não controla.
- **Regras de validação em formato de valor: `ArrayKeys` e `Distinct`.** Uma
  nova trait `ValueRule` (`passes(&self, value: &serde_json::Value)`) fica
  ao lado de `Rule`, compartilhando o mesmo contrato de mensagem por chave.
  O `rules::ArrayKeys(&[...])` recusa um objeto JSON que carregue qualquer
  chave fora da lista permitida (o `array:keys` do Laravel, #60918); o
  `rules::Distinct { ignore_case, strict }` recusa um array JSON com um
  elemento repetido (o `distinct` do Laravel). As linhas de `validate!`
  aceitam qualquer um dos dois tipos de regra na mesma lista de campos - o
  despacho é automático, escolhido pela trait que a regra implementa, e não
  por uma nova sintaxe de linha.
- **`Job::delay()`** - jobs podem declarar um atraso padrão
  (`fn delay() -> Option<Duration>`, padrão `None`), honrado pelo
  `Queue::push` e pelo `Queue::bulk`: o `available_at` passa a ser
  `now + delay` em vez de `now`. Um atraso explícito no local de chamada
  ainda vence - o `Queue::push_later(job, at)` e o
  `Queue::later(delay, job)` usam o timestamp do chamador literalmente e
  nunca consultam o `Job::delay()`.
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`.**
  Uma notificação em fila (`Notify::queue`) agora leva seus próprios padrões
  de ajuste de fila para cada push de `SendNotificationJob` por canal,
  através do primitivo `EnvelopeOverrides` que o `Mail::on_queue` usa - o
  `fail_on_timeout(&self) == true` manda para dead-letter no primeiro
  timeout em vez de tentar de novo, acompanhando o atributo de notificação
  `#[FailOnTimeout]` do Laravel (#61072). Todos os cinco assumem por padrão
  os valores de `Job` que o `SendNotificationJob` já tinha, então uma
  notificação que não sobrescreve nada não é afetada.
- **`Mail::on_queue` / `Mail::on_connection` +
  `Queue::push_with`/`later_with`.** Um mailable em fila agora se roteia com
  `Mail::to(..).on_queue("emails").queue(mailable)`, ou usa o padrão via
  `Mailable::queue(&self)`. Os dois superam qualquer `Queue::route`
  registrada para o job e o próprio `Job::queue()`/`Job::connection()` do
  job - o novo primitivo `EnvelopeOverrides` por trás deles
  (`Queue::push_with(job, overrides)` /
  `Queue::later_with(delay, job, overrides)`) também cobre timeout,
  fail-on-timeout, max-tries e backoff para um push. Os snapshots em fila do
  `MailFake` agora carregam a `queue` resolvida, com `queued_on(...)` /
  `assert_queued_on(name, queue)` para fazer a asserção.
- **`Application::http_bootstrap(f)`** - um hook de boot só para HTTP. Ele
  roda depois do `bootstrap` e apenas no caminho `serve` / `web:run`, então
  os workers de fila, de agendamento e de workflow e o binário de console
  nunca o rodam. Imagens de contêiner de worker e de console não precisam
  mais de um manifest de frontend construído para dar boot: o
  `Inertia::install` falha fechado em produção quando ele falta, e essa
  verificação agora só roda em um processo que de fato serve HTTP.
- **`Router::inertia(path, component, props)`** - o `Route::inertia` do
  Laravel, para uma página estática cujo handler seria de uma linha.
  Registra `GET` (o HEAD cai nele) e devolve um `RouteBuilder`, então a rota
  pode receber nome e middleware. O `Router::view` é mantido como alias.
- **Opções de envio do SES v2.** O transporte SES agora emite `TenantName`,
  `ConfigurationSetName` e `ListManagementOptions` no `SendEmail`. Cada um
  tem um padrão de nível de transporte (`SesMailTransport::tenant_name` /
  `configuration_set_name` / `list_management`) e uma sobrescrita por
  mensagem via header (`X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`,
  `X-SES-LIST-MANAGEMENT-OPTIONS`), com o header vencendo. Os headers são
  consumidos quando a solicitação é montada e nunca são renderizados na
  mensagem.
- **`without_cookies` em todo builder de resposta.** O `HttpResponse`, o
  `Response` (via `ResponseExt`), o `Redirect` e o `RedirectRouteBuilder`
  agora expiram uma lista de cookies em uma só chamada, e o `Redirect` /
  `RedirectRouteBuilder` ganharam o `without_cookie` de nome único que lhes
  faltava. O novo `Cookie::forget_with(name, path, domain)` constrói um
  cookie de exclusão delimitado ao caminho e ao domínio com que o original
  foi definido - um `forget` simples nunca limpa um cookie definido fora de
  `/`.
- **O `Queue::fake()` carimba um id de envelope em todo push capturado.** O
  `pushed_with_id::<J>()` devolve pares `(job, id)`, e o fake agora despacha
  o mesmo par `JobQueueing` / `JobQueued` que um push de driver real
  despacha - carregando aquele id - então um teste pode correlacionar um
  push capturado com o que seus listeners viram. Os helpers de fake
  existentes não mudam.
- **Evento de fila `UniqueJobSkipped`.** O `Queue::push_unique` agora
  despacha
  `queue::events::UniqueJobSkipped { job_name, unique_id, connection }`
  quando suprime uma duplicata, então uma deduplicação é observável em vez
  de silenciosa. O valor de retorno da chamada não muda (`Ok(false)`).
- **`model_keys()` no construtor de queries e em coleções.** O
  `User::query().model_keys().await?` devolve a chave primária de cada linha
  correspondente sem hidratar um único model, projetando a chave qualificada
  pela tabela (`users.id`), de modo que a query sobreviva a um join. O
  `Collection::model_keys()` é a contraparte já hidratada. O
  `#[suprnova::model]` agora também declara o tipo Rust da chave como
  `EloquentModel::Key`, então os dois devolvem o tipo que `key_type` nomeia
  em vez de um turbofish escolhido pelo chamador.

### Corrigido

- **Soft deletes no PostgreSQL agora usam placeholders cientes do backend, e
  as escritas de timestamp geradas honram os casts declarados.** O
  `delete()` e o `restore()` renderizam placeholders ordinais do PostgreSQL
  em vez dos placeholders `?` do MySQL e do SQLite. As escritas geradas de
  create, update, save, touch e soft delete também convertem timestamps pelo
  tipo de armazenamento do `Cast` declarado de cada campo, então colunas
  `TIMESTAMPTZ` nativas não recebem mais valores em texto. Obrigado a
  [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) por relatar
  os dois defeitos e enviar uma correção no
  [PR #3](https://github.com/eas4ai/suprnova/pull/3).
- **As execuções padrão do gate do workspace e do Magnetar não exigem mais
  serviços PostgreSQL ou MySQL ativos.** As suítes de comportamento
  específicas de backend são testes de qualificação explícitos e ignorados,
  que ainda falham quando invocados deliberadamente sem o banco de dados
  configurado. Testes que só checavam alcançabilidade e requisitos
  permanentes de ambiente do gate foram removidos, então mudanças não
  relacionadas não pagam por configuração de banco de dados externo em toda
  execução de verificação.

- **O `PartialFilter::narrow` agora é `pub`.** Seus quatro predicados irmãos
  (`should_include`, `should_include_eager`, `should_include_optional` e o
  próprio tipo) já eram públicos, mas a passagem de estreitamento que torna
  correta a resposta `true` de `should_include_eager` - aparar um valor
  resolvido até os caminhos com ponto que uma entrada `only`/`except` de
  fato pediu - era `pub(crate)`. Um chamador que montasse tratamento próprio
  de reload parcial sobre o `PartialFilter` não tinha jeito público de
  reproduzir aquele estreitamento e acabaria enviando um valor inteiro sob
  uma entrada `only` com ponto, mesmo com o `should_include_eager` relatando
  a chave como incluída.
- **O `QueuedSnapshot` do `MailFake` agora consegue fazer asserção sobre
  `.on_connection(...)`.** O `Queue::fake()` ganhou o
  `assert_pushed_on_connection` na Wave 3, ao lado do
  `assert_pushed_on_queue`; o `Mail::fake()` só ganhou a metade da fila,
  então um mailable enfileirado com uma sobrescrita de conexão era resolvido
  e aplicado ao despacho real, mas não podia ser verificado pelo fake. Os
  novos `QueuedSnapshot::connection`, `MailFake::queued_on_connection` e
  `MailFake::assert_queued_on_connection` fecham a lacuna, espelhando o
  formato do `assert_queued_on`.
- **Uma prop compartilhada com ponto era inalcançável por uma entrada `only`
  nua.** O `App::inertia_share("auth.user", …)` seguido de
  `router.reload({ only: ['auth'] })` devolvia `props: {"errors":{}}` - o
  compartilhamento sumia de vez. O registro guarda `auth.user` como uma
  única chave literal, e a passagem de aninhamento por `Arr::set` só a
  aninha depois que toda prop resolveu, então o portão de reload parcial via
  a chave ainda plana e não a casava nem com `auth` nem com nada. As
  entradas `only`/`except` agora são simétricas: uma entrada pode nomear a
  chave de uma prop exatamente, um caminho *dentro* dela (`user.name`, que
  estreita) ou um **ancestral** dela (`auth` contra a chave `auth.user`, que
  envia a prop inteira, porque o chamador pediu a raiz inteira). Um
  `except: ['auth']` nu descarta toda chave de prop abaixo dele do mesmo
  jeito que o `Arr::forget` descarta a subárvore inteira no conjunto já
  aninhado do Laravel. O prefixo precisa terminar em uma fronteira de
  segmento, então uma prop `authAgent.user` não relacionada fica intocada
  pelas duas listas. O Laravel nunca esbarra nisso porque o `Inertia::share`
  roda `Arr::set` na hora do compartilhamento; o registro do Suprnova não
  pode, já que um compartilhamento lazy não tem valor para aninhar até a
  solicitação resolvê-lo.
- **Um campo `#[data(lazy(deferred))]` driblava a lista de permissão de
  `?include=`.** O caminho de resolução marcado pelo dono em `resolve_props`
  selecionava props com `Prop::is_lazy()`, que é falso para qualquer coisa
  que carregue uma flag - e um campo deferred é `Visibility::Deferred`. O
  campo portanto resolvia pelo caminho de prop comum, onde não existe
  verificação do conjunto de includes, e era enviado a qualquer cliente que
  mandasse o follow-up de deferred, independentemente de a solicitação ter
  optado por incluir o campo. O `Prop::resolve_with_owner` agora bloqueia
  toda prop marcada pelo dono e respaldada por resolver, com flags ou sem, e
  o `resolve_props` roda esse bloqueio antes de qualquer outro bloco: um
  campo fora de `?include=` é descartado inteiro (sem valor, sem anúncio em
  `deferredProps`), e um campo nomeado por `?include=` mas fora da lista de
  permissão do DTO levanta seu `400` antes que o `X-Inertia-Partial-Data`
  possa absorvê-lo. Não é uma regressão - o código anterior à Wave 4
  bloqueava pela variante de enum `Prop::Lazy`, que um `Prop::Defer` também
  não satisfazia - mas era um buraco real de qualquer forma.
- **O `deferredProps` era reanunciado em um reload parcial que casava.** Um
  reload parcial que nomeava uma chave deferred ainda anunciava de volta ao
  cliente todas as *outras* chaves deferred, que ele então buscava de novo,
  e de novo no reload parcial seguinte. O `resolveDeferredProps` do Laravel
  devolve `[]` no momento em que a solicitação é parcial, antes de
  inspecionar uma única prop (`Response.php:661-663`); o bloco agora é
  descartado inteiro em qualquer reload parcial que case. Um reload parcial
  mirado em outro componente é uma visita comum para este portão, como para
  todos os outros, então os anúncios dele não são afetados.
- **O conjunto `errors` filtrava de formas diferentes conforme a origem dos
  erros.** O conjunto vindo do flash da sessão é semeado antes do laço de
  resolução e nenhum filtro de reload parcial conseguia alcançá-lo, enquanto
  o `.with("errors", …)` do próprio handler passava pelos portões comuns -
  então `only: ['errors.email']` enviava o conjunto semeado inteiro, mas
  apenas um conjunto de um campo quando ele vinha do handler, e
  `only: ['users']` substituía o conjunto do handler pelo semeado em vez de
  deixar a chave em paz. Os dois caminhos agora tratam `errors` como sempre
  visível, acompanhando o middleware do Laravel, que o compartilha como
  `Inertia::always(...)` e reinjeta o valor cru através de `resolveAlways`
  depois da reconstrução de `only`/`except`. Esse é o formato de que o
  cliente precisa: ele dobra uma resposta parcial com
  `{...current.props, ...response.props}`, então um objeto `errors` vazio
  apaga mensagens que já estão na tela, enquanto um não filtrado as deixa
  corretas. Uma flag de visibilidade explícita na chave ainda vence, então
  `.prop("errors", Prop::eager(…).optional())` se comporta de forma
  opcional.
- **O `Queue::fake()` agora consegue observar `EnvelopeOverrides` por
  push.** Um job enviado por `Queue::push_with`/`Queue::later_with` era
  indistinguível de um `Queue::push` simples sob o fake - o `FakePush`
  carregava só o payload e o `available_at`, então a sobrescrita nunca saía
  da facade e nada conseguia afirmar que um teste despachou para a fila ou a
  conexão certa. O novo
  `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>`
  devolve cada push capturado emparelhado com o que ele declarou; o
  `assert_pushed_on_queue::<J>(queue)` e o
  `assert_pushed_on_connection::<J>(connection)` cobrem o caso comum de um
  só campo, espelhando o `MailFake::assert_queued_on`. Todo outro ponto de
  entrada (`push`, `push_later`, `bulk`, `push_unique`, os despachantes de
  chain/batch) continua sem receber sobrescritas e registra
  `EnvelopeOverrides::default()`, então um push simples se lê sob o fake
  exatamente como "nenhuma sobrescrita declarada".
- **Um worker de SSR que travava no meio do corpo da resposta podia pendurar
  uma renderização para sempre.** O `SsrConfig::timeout` limitava apenas a
  espera pelos headers da resposta; uma vez que os headers chegavam, ler o
  corpo não tinha timeout próprio, então um worker que aceitava a conexão,
  mandava os headers e então parava de mandar dados deixava a solicitação
  pendurada além do timeout configurado, em vez de recair para CSR (ou dar
  erro, sob `ssr_throw_on_error`). As duas fases agora compartilham um único
  prazo, então o timeout configurado limita a chamada de SSR inteira, como a
  documentação dele já prometia.
- **Cookies enfileirados - incluindo o cookie de remember-me que o
  `Auth::login_remember` define - eram descartados em silêncio em três
  caminhos internos de falha fechada do `SessionMiddleware`.** Uma falha de
  leitura de sessão, uma falha de escrita de sessão e uma falha de
  criptografia do cookie de sessão retornavam cada uma um `500` sintetizado
  direto, pulando a drenagem de cookies pendentes que roda no fim do
  `handle`. Qualquer coisa enfileirada via `Cookie::queue` naquela
  solicitação - incluindo uma linha de token de remember-me já commitada no
  banco de dados - nunca chegava ao cliente como um header `Set-Cookie`. Os
  três caminhos agora drenam os cookies pendentes antes de retornar, igual a
  um erro devolvido pelo handler ou a um redirecionamento. Isso não cobre um
  panic não capturado, assim como os próprios cookies enfileirados do
  Laravel se perdem em um.
- **O `Queue::push_unique` agora honra o `Job::delay()`, acompanhando o
  `Queue::push`, o `Queue::push_with` e o `Queue::bulk`.** Ele antes
  calculava o `available_at` direto de `Utc::now()`, então um job que
  declarava um atraso padrão (`fn delay() -> Option<Duration>`) era
  despachado imediatamente quando enviado por `push_unique`, em vez de
  depois daquele atraso. O `Queue::push_unique_later` e o
  `Queue::later_unique` não são afetados - eles já recebem um timestamp ou
  um atraso explícito do chamador e nunca consultam o `Job::delay()`, a
  mesma regra que `push_later`/`later` seguem.

### Alterado

- **O branch de desenvolvimento atual usa o SeaORM 2.0 e exige o Rust
  1.94.0.** O Suprnova preserva os formatos de código de Eloquent, de
  `#[model]`, de migração e da facade de banco de dados. Aplicações que
  chamam o SeaORM diretamente precisam importar `ExprTrait` para os métodos
  de expressão do SeaQuery e usar os métodos de conexão `*_raw` explícitos
  para valores `Statement` pré-montados. O SeaQuery agora é 1.0, e o driver
  de vetor MariaDB direto usa o SQLx 0.9. Bancos de dados existentes não
  exigem migração de dados na aplicação; schemas PostgreSQL novos mantêm
  chaves primárias baseadas em serial.
- **Mais três dependências não usadas removidas.** O `pretty_assertions` e o
  `qrcode` saem do crate do framework (o `totp-rs` já carrega a feature
  `qr`, então o provisionamento de QR para o cadastro de dois fatores não é
  afetado), e o `notify-debouncer-mini` sai da CLI (o `notify` em si fica -
  os observadores de `serve` e de `generate-types` o usam diretamente). Os
  três foram confirmados como não usados pelo `cargo-udeps` mais uma busca
  por todo o código-fonte que cobre doc tests.
- **O `suprnova-macros` não depende mais de `serde` nem de
  `serde_derive_internals`.** Nenhum dos dois era usado: os caminhos
  `::serde::Serialize` que as macros emitem resolvem no crate consumidor,
  não no próprio crate de macros. Sem efeito no código gerado.
- **O `match_on` do `MergeStrategy` agora carrega mais de um nome de
  campo.** O `Append`, o `Prepend` e o `Deep` passam cada um de
  `match_on: Option<String>` para `match_on: Option<Vec<String>>`, então o
  `InertiaResponse::merge_with` / `merge_lazy_with` podem deduplicar por
  vários campos do mesmo jeito que o
  `.prop(key, Prop::eager(v).match_on([...]))` já podia - antes disso, os
  atalhos do builder de resposta eram estritamente menos expressivos do que
  montar uma `Prop` diretamente. Veja Atualizando.
- **Props de scroll agora emitem semântica de `reset` e de merge idêntica à
  do Laravel.** O `scrollProps[key].reset` é `true` exatamente quando o
  cliente nomeou `key` em `X-Inertia-Reset`, acompanhando o
  `resolveScrollProps` do Laravel - e não `true` em toda visita sem um
  header `X-Inertia-Infinite-Scroll-Merge-Intent`, como antes. Uma prop de
  scroll agora também carrega metadados de merge incondicionalmente, com
  append como padrão: uma visita nova (sem header nenhum) emite
  `reset: false` mais uma entrada em `mergeProps`, onde antes emitia
  `reset: true` e nenhum metadado de merge. Uma chave em `X-Inertia-Reset`
  fica excluída de `mergeProps` / `prependProps` naquela resposta, a mesma
  exclusão que uma prop de merge comum já tinha.
- **O `ssr:check` agora verifica se a rota `GET /health` do worker de SSR
  responde 2xx**, em vez de apenas confirmar que alguma coisa aceitou uma
  conexão TCP. Todo worker de `@inertiajs/{vue3,react,svelte}/server`
  responde `/health` de fábrica, então isso não exigiu mudança nenhuma do
  lado do worker - acompanha o `Inertia\Ssr\HttpGateway::isHealthy()` do
  Laravel.
- **A prop `errors` do Inertia agora carrega uma string por campo, não um
  array.** Um conjunto de validação vindo do flash da sessão é renderizado
  como `{ email: "The email field is required." }` em vez de
  `{ email: ["The email field is required."] }`, acompanhando o padrão do
  Laravel e o próprio `ErrorValue = string` do Inertia. O
  `InertiaConfig::with_all_errors(true)` restaura o formato de array. Uma
  prop `errors` que o próprio handler define é repassada intocada, e o flash
  da sessão (`Redirect::with_errors`, `session.pull_errors_flash()`)
  continua guardando arrays - só a prop de página renderizada muda.
- **O `Model::TOUCHES` saiu de um const inerente para o `EloquentModel`.** A
  cascata de toque no pai vive em um padrão da trait `Model`, e um padrão de
  trait não consegue ler um const inerente. O `Comment::TOUCHES` ainda
  resolve - agora ele precisa de `use suprnova::EloquentModel;` no escopo.
  Models sem um atributo `touches` recebem o padrão vazio da trait.
- **O `RelationEntry` ganhou `related_updated_at_column`.** Qualquer coisa
  que construa um `RelationEntry` à mão precisa do campo extra; nada na
  árvore faz isso, a macro emite todos eles.
- **O `Router::view` agora recusa props que não sejam um objeto JSON.**
  Antes ele as ignorava em silêncio, registrando uma rota que renderizava um
  conjunto de props vazio sem diagnóstico nenhum. O `null` continua aceito
  como "sem props"; o `Router::try_inertia` é a forma falível.
- **A versão de assets do Inertia agora usa por padrão um hash do manifest
  de build do Vite** em vez do literal `"1.0"`, então um deploy invalida
  clientes de vida longa sem ninguém precisar lembrar de atualizar uma
  string. O `InertiaConfig::manifest_path(...)` reaponta o resolvedor com
  ele; um `.version(...)` / `.version_with(...)` explícito ainda vence. Sem
  manifest em disco - desenvolvimento local - a versão recai para `"1.0"`,
  que é o que toda aplicação via antes, então nada muda até você fazer um
  build. O novo `VersionResolver::from_manifest(path)` expõe o resolvedor
  diretamente.

### Obsoleto

- **O `Cookie::read_encrypted` agora é o leitor legado só de v1.** Código
  que cunha com `Cookie::encrypted` e lê com `read_encrypted` falha em tempo
  de execução no primeiro valor escrito depois deste release; mude para
  `read_encrypted_for(name, wire)`. Os pontos de entrada não
  contextualizados de `CryptPurpose::Cookie` também estão superados. As duas
  remoções estão agendadas para a 1.4.0.

### Atualizando
- **Avisos de decriptação de cookie agora têm dois eixos independentes.** Um
  aviso `KeyOrigin::Previous(index)` significa recriptografar o valor sob a
  `APP_KEY` atual e remover aquela chave anterior só depois que a cauda da
  rotação tiver passado; um aviso `AadVersion::Legacy` significa reemitir o
  cookie pela API vinculada ao nome antes da remoção do fallback na 1.4.0.
  Um valor pode reportar os dois.
- **O `SESSION_COOKIE_PREFIX` é opt-in.** Implante `__Host-` só com HTTPS,
  `SESSION_SECURE=true`, `SESSION_PATH=/` e sem `SESSION_DOMAIN`; scaffolds
  locais em HTTP o deixam vazio. O `with_session_config` do `CsrfMiddleware`
  mantém o nome literal `XSRF-TOKEN`; use
  `.xsrf_cookie_name("__Host-XSRF-TOKEN")` quando um cliente estiver
  configurado para esse nome separado.
- **O `DecryptOrigin` agora é uma struct `#[non_exhaustive]` de dois
  eixos.** Leia os campos `key` e `aad` de forma independente e mantenha uma
  estratégia de match compatível com wildcard para os enums `KeyOrigin` /
  `AadVersion`.
- **O `SessionConfig` e o `CookieOptions` agora são `#[non_exhaustive]`.**
  Literais de struct e atualizações funcionais de registro no código da
  aplicação precisam migrar para `Type::default()` seguido de atribuições a
  campos públicos ou de métodos de builder.

- **O `FrameworkError` agora é `#[non_exhaustive]`.** Um `match` sobre ele
  no seu próprio código precisa de um braço wildcard. Este é o último
  release em que acrescentar uma variante teria sido uma mudança
  incompatível.
- **O campo `match_on` de `MergeStrategy::Append`/`Prepend`/`Deep` agora é
  `Option<Vec<String>>`, não `Option<String>`.** Um local de chamada que
  constrói a forma de literal de struct diretamente -
  `MergeStrategy::Append { match_on: Some("id".into()) }` - não compila
  mais; embrulhe o nome do campo em um `Vec`: `Some(vec!["id".into()])`. O
  `match_on: None` não é afetado e não precisa de mudança.
- **Um reload parcial que casa não emite mais `deferredProps`.** Código que
  lê `page.deferredProps` de uma resposta de reload parcial - um componente
  próprio de carregamento deferred, um snapshot de teste, uma asserção ponta
  a ponta - agora vai achar a chave ausente onde ela costumava listar as
  props deferred que a solicitação não nomeou. Leia os anúncios na visita
  inicial (não parcial), que é onde o Laravel os coloca e onde o cliente
  oficial os lê.
- **Uma entrada `except` nua agora descarta chaves de prop com ponto abaixo
  dela.** O `X-Inertia-Partial-Except: auth` antes deixava na resposta uma
  prop registrada sob `auth.user`, porque o portão comparava chaves
  inteiras. Agora ela é descartada. Se uma página dependia de uma entrada
  `except` nua podar apenas a chave exata, nomeie a chave exata
  (`except: ['auth.user']`) ou estreite com um caminho com ponto.
- **O `errors` ignora `only`/`except`.** Um reload parcial que filtrava para
  fora uma prop `.with("errors", …)` fornecida pelo handler, ou que a
  estreitava com uma entrada com ponto, agora a envia inteira. Testes que
  afirmam um objeto `errors` fatiado ou vazio em um reload parcial precisam
  ser atualizados. Para manter o conjunto fora de uma resposta
  deliberadamente, marque-o - `.prop("errors", Prop::eager(…).optional())` -
  em vez de contar com as listas de reload parcial.
- **O `Prop::resolve_with_owner` também bloqueia props com flags.** Ele
  antes resolvia qualquer prop que não fosse `Prop::is_lazy()` - um valor
  eager *ou* um resolver carregando uma flag - sem consultar o conjunto de
  includes. Agora ele bloqueia toda prop respaldada por resolver e só deixa
  passar sem bloqueio um valor já materializado. Um campo
  `#[data(lazy(deferred))]` consequentemente precisa de `?include=<field>`
  na solicitação antes de resolver ou de ser anunciado, igual a todo outro
  sabor de lazy. Acrescente o campo à lista `?include=` da solicitação, ou
  remova o atributo `lazy(...)` se ele nunca foi feito para ser opt-in.
- **O `reset` de uma prop de scroll não segue mais o header de intenção de
  merge.** Código que lê `page.scrollProps[key].reset` diretamente - um
  componente próprio de scroll infinito, um snapshot de teste - vai ver
  `reset: false` (mais uma entrada em `mergeProps`) em uma revisita simples
  que antes lia `reset: true` e não carregava metadado de merge. O
  componente oficial `<InfiniteScroll>` se comporta de forma diferente
  apenas em uma revisita simples: ele escuta `reset` em todo evento
  `success` do `router`, não só em um `router.reload()` explícito, então uma
  revisita normal não limpa mais o estado acumulado dele a menos que o
  servidor de fato tenha nomeado a chave em `X-Inertia-Reset`, o que
  acompanha o Laravel. Mande `X-Inertia-Reset: <key>` explicitamente onde
  quer que o antigo comportamento de "qualquer visita que não seja
  append/prepend reseta" fosse usado.
- **O `Prop::match_on` recebe `impl MatchOnFields`, não
  `impl Into<String>`.** O novo bound é o que deixa uma chamada nomear
  vários campos (`match_on(["id", "slug"])`), e sua lista de impls é
  deliberadamente fechada - só `&str`, `String`, `[T; N]` e `Vec<T>`. Um
  impl abrangente sobre `IntoIterator` não está disponível: a coerência o
  rejeita contra os impls de `&str` e `String`, já que nada impede esses
  tipos de ganharem um impl de `IntoIterator` depois. Três tipos de
  argumento que compilavam antes não compilam mais: `&String`,
  `Cow<'_, str>` e `Box<str>`. Passe um `&str` no local de chamada -
  `match_on(name.as_str())` para um `&String`, `match_on(name.as_ref())`
  para um `Cow<'_, str>`, `match_on(&*name)` para um `Box<str>`.
- **Uma entrada `only`/`except` com ponto agora estreita a prop de nível
  superior em vez de excluí-la por completo.** Antes desta correção, o
  `X-Inertia-Partial-Data: user.name` fazia o `should_include_eager`
  procurar uma entrada `"user"` de correspondência exata, não achava nenhuma
  e descartava em silêncio a prop `user` inteira - um cliente que pedia um
  campo de `user` não recebia nada. Qualquer componente de página do
  frontend que por acaso dependesse dessa lacuna (tratando um
  `router.reload({ only: [...] })` com ponto como equivalente a omitir a
  chave) agora recebe `{ user: { name: ... } }` em vez disso. Nenhuma
  mudança de código é necessária - isso é o que o protocolo do Inertia v3 já
  especifica que o contrato de solicitação/resposta significa. A mesma
  correção se aplica ao `should_include_optional`, e o efeito dela é
  operacionalmente maior: uma entrada `only` com ponto (`permissions.read`)
  agora conta como um pedido explícito da chave de nível superior de uma
  prop `Optional` ou `Defer`, o que antes exigia uma entrada nua
  (`permissions`) para disparar. Uma solicitação que antes pulava o resolver
  daquela prop por completo agora o roda - se o resolver bate em um banco de
  dados ou em um serviço externo, um cliente que já manda solicitações de
  reload parcial com ponto começa a emitir esse trabalho em solicitações que
  antes não faziam nenhum. Observe o volume de chamadas de resolver depois
  de atualizar, se sua aplicação tem props `Optional`/`Defer` com tráfego de
  reload parcial com ponto.
- **O `InertiaSharedData::share` agora recebe o nome do componente da
  página.** Acrescente um parâmetro `component: &str` depois de `req`:
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```
  Ignore-o (`_component`) se seu provider não precisar variar por página - o
  `RenderContext` do Laravel carrega o mesmo par (`component`, `request`)
  para o `ProvidesInertiaProperties::toInertiaProperties`.
- **A `Prop` é uma struct, não um enum.** As variantes dela sumiram;
  construa e leia props através de métodos:
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)` (o `Prop::lazy(closure)` não muda)
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`; `matches!(prop, Prop::EagerNone)` ->
    `prop.is_absent()`
  As structs de payload `DeferConfig`, `MergeConfig`, `OnceConfig` e
  `ScrollConfig` foram removidas - os campos delas agora são flags na
  `Prop`. O `Prop::is_deferred()` foi renomeado para `Prop::has_resolver()`,
  que é o que ele sempre quis dizer. O `DeferOptions`, o `OnceOptions`, o
  `MergeStrategy`, o `ScrollMetadata` e todo método de builder de
  `InertiaResponse` não mudam, então uma aplicação que só usa o builder de
  resposta não precisa de edições. Aplicações que montam props à mão -
  tipicamente uma implementação de `InertiaSharedData` - precisam das
  renomeações acima.

- **Esta correção protege sessões que você já tem, não só solicitações daqui
  para frente.** Atualizar já basta: um cookie de sessão escrito por um
  release anterior pode carregar um `_previous.url` que nunca foi
  sanitizado, e o `SessionData::previous_url()` agora o descarta na leitura,
  na primeira vez que aquela sessão for usada depois da atualização, em vez
  de confiar nele porque já está guardado. Você não precisa invalidar
  sessões existentes, migrar a tabela de sessão nem forçar um novo login.
  Uma solicitação cujo caminho pareça relativo ao protocolo (`//host`)
  também não atualiza mais a URL anterior registrada daqui para frente - se
  a rota `fallback!` da sua aplicação (ou qualquer rota que responda 200 e
  seja alcançável por um caminho incomum) algum dia dependeu legitimamente
  de tal caminho virar o destino do `Redirect::back()`, ela não depende
  mais. De qualquer forma, o valor anterior e seguro na sessão é mantido no
  lugar (ou o fallback do próprio `Redirect::back(fallback)` vence, se nada
  seguro tiver sido registrado). Nenhuma mudança de código é necessária, a
  menos que você dependesse exatamente do caso extremo que isso fecha, que
  já era um risco de open redirect.
- **Remova o `[0]` de toda ligação `errors.<field>` nas suas páginas.** Com
  o novo formato padrão, `errors.email` é uma string, então
  `errors.email[0]` renderiza o primeiro caractere dela em vez da mensagem.
  Mude o tipo TypeScript de `string[]` para `string` na mesma hora. Se você
  preferir não mexer nas suas páginas, defina
  `InertiaConfig::with_all_errors(true)` na config que você passa para o
  `Inertia::install` e acrescente a ampliação de módulo
  `errorValueType: string[]` para `@inertiajs/core`. Os frontends starter já
  vêm com o novo formato.
- **Um handler que fazia à mão o redirecionamento de volta depois de uma
  falha de validação pode apagá-lo.** A ponte é automática agora; um handler
  que ainda redireciona por conta própria continua funcionando, porque o
  middleware só age sobre um `422` que carrega um objeto `errors`
  preenchido.
- **Um filho do `suprnova serve` que sofre crash agora respawna em vez de
  encerrar a sessão.** Se você dependia de um crash parar o `suprnova serve`
  de vez (um smoke test na CI, um script que trata a saída como "algo está
  errado"), passe `--no-restart` para restaurar exatamente esse
  comportamento. As tentativas também são limitadas por padrão: um processo
  que sofre crash 5 vezes seguidas para de ser tentado de novo (aumente o
  limite com `--restart-tries`, ou use `--no-restart` para o comportamento
  original de um crash e pronto).
- **O `Model::TOUCHES` não é mais um const inerente.** Código que lia
  `Comment::TOUCHES` diretamente precisa de `use suprnova::EloquentModel;`
  (ou `suprnova::eloquent::EloquentModel`) no escopo - o const foi para lá
  para que a cascata de toque no pai, um padrão da trait `Model`, consiga
  lê-lo. Um `grep -rn TOUCHES` sobre a sua aplicação acha todos os locais de
  chamada; a maioria das aplicações não tem nenhum, já que o const antes não
  fazia nada em tempo de execução.
- **O `RelationEntry` ganhou um campo.** Só código que constrói um
  `RelationEntry` à mão precisa de mudança - acrescente
  `related_updated_at_column` ao literal. Os registros de relação gerados
  por macro que o framework traz já o emitem, então uma aplicação comum, que
  não faz nada além de declarar relações através de `#[suprnova::model]`,
  não é afetada.
- **O `Router::view` com props que não são objeto agora entra em panic no
  boot.** Antes ele registrava em silêncio com um conjunto de props vazio; o
  `view` delega ao `Router::inertia`, que exige um objeto (ou `null`) e
  entra em panic caso contrário. Se uma chamada de `view` puder carregar
  props que não sejam objeto, mude para o `Router::try_inertia` e trate o
  `Err` - fora isso, nada muda para você.
- **O padrão da versão por manifest do Inertia pode mudar sua string de
  versão no momento em que um build existir.** Uma aplicação ou um teste que
  fixa `X-Inertia-Version: 1.0` continua funcionando só até um manifest do
  Vite aparecer em disco; assim que um aparece, a versão vira o hash do
  manifest. Se você precisa da constante antiga, leia-a você mesmo do
  `VersionResolver::from_manifest(path)` ou fixe `.version(...)`
  explicitamente. Espere que o primeiro deploy depois da atualização force
  um ciclo de reload de página inteira para clientes já conectados - uma vez
  só, e é esse o objetivo da mudança. O valor de fallback para quando não há
  manifest é exportado como `suprnova::MANIFEST_VERSION_FALLBACK`, então
  você nunca mais precisa fixar `"1.0"` no código.
- **Mova o registro de `Inertia::install` e de `global_middleware!` para
  fora do `bootstrap::register`.** Coloque-os em uma função nova e passe-a
  para o `.http_bootstrap(...)` - o novo formato do scaffold é um
  `register_http_stack()` síncrono, chamado como
  `.http_bootstrap(|| async { bootstrap::register_http_stack() })`.
  Aplicações que pularem isso mantêm o comportamento de hoje, falha de boot
  do worker por manifest de frontend faltando incluída.

## 1.2.4 - 2026-08-18

### Segurança

- **O segredo de bypass do modo de manutenção é comparado em tempo
  constante.** O `MaintenanceMiddleware` casava a URL do segredo com uma
  comparação de string simples, que retorna no primeiro byte diferente.
  Como o segredo é uma credencial bearer carregada no path da
  solicitação, essa diferença de tempo dizia a um atacante o tamanho do
  prefixo que ele tinha acertado. A comparação agora percorre o
  comprimento completo em bytes via `subtle::ConstantTimeEq`, fazendo
  curto-circuito somente numa diferença de comprimento - o mesmo formato
  da comparação de cookie de bypass ao lado dela.

- **`rules::Url` agora rejeita URIs de script.** A regra aceitava
  qualquer esquema que `url::Url` conseguisse interpretar, incluindo
  `javascript:` e `vbscript:`, então uma URL validada ainda podia ser um
  sink de execução de script ao ser renderizada em um `href`. Ela agora
  aplica o formato da regra `url` do Laravel (o padrão
  `^(PROTOCOLS)://HOST` de `Illuminate\Support\Str::isUrl`): o esquema
  precisa estar na allowlist do Laravel, ser seguido por `://`, **e** ser
  seguido por um host não vazio - o grupo de host do Laravel não tem `?`,
  então um host ausente ou vazio nunca casa, mesmo com um esquema
  listado. A lista de esquemas e a exigência de `://` mais host são as do
  Laravel, ao pé da letra; o host em si é interpretado pelo crate `url`
  em vez do regex do Laravel, então alguns casos de borda ainda diferem -
  uma porta fora do intervalo é rejeitada aqui e aceita lá, e hosts IDN
  normalizam de forma diferente. O novo `Url::protocols(&[...])` espelha
  o `url:http,https` do Laravel; `HttpUrl` agora é açúcar sintático
  literal para ele e mantém a própria mensagem. **Mudança de
  comportamento:** uma URL com um esquema não listado que antes validava
  agora falha - nomeie o esquema com `Url::protocols(&["myapp"])` se a
  intenção era aceitá-lo. Mais duas mudanças de comportamento: `mailto:`,
  `data:`, e `tel:` estão na allowlist do Laravel pelo nome, mas não
  carregam componente de autoridade, então agora falham; e paths no
  estilo `file:///etc/passwd` - `scheme://` sem nada entre as duas
  últimas barras - agora também falham, já que uma string vazia também
  não é um host. As duas decorrem da própria regra de `://` mais host do
  Laravel.

- **Respostas Inertia agora anunciam `Vary: X-Inertia` em todo lugar.** O
  header era definido apenas nas próprias respostas de objeto de página.
  Redirects, 404s, 422s, e respostas estáticas não carregavam nenhum,
  então um cache compartilhado chaveado só pela URL podia servir o objeto
  de página JSON para uma navegação dura do navegador, ou o shell HTML
  para um XHR do Inertia. O novo `InertiaHeadersMiddleware` - registrado
  por `Inertia::install` como o mais externo dos três - o define em toda
  resposta, e transforma um `200` vazio numa visita Inertia em um `303`
  de volta, em vez de uma resposta que o cliente rejeita como não
  Inertia. O `InertiaVersionMiddleware` agora refaz o flash da sessão
  antes do seu `409`, para que um erro flashado sobreviva ao GET de
  página inteira de acompanhamento do cliente.

- **Três correções de resposta Inertia.** `InertiaResponse::location_for(&req, url)`
  retorna `409` + `X-Inertia-Location` para um XHR do Inertia e um `302` + `Location` simples para uma navegação dura, então um bounce de OAuth
  ou SSO iniciado fora da SPA não termina mais num beco sem saída com um
  `409` sem corpo. O `location(url)` existente mantém seu formato
  sempre-`409`. O novo `App::clear_history()` faz flash da flag de
  limpeza de histórico na sessão, para que ela sobreviva ao redirect de
  logout e chegue à página que de fato renderiza - o `.clear_history()`
  por resposta marcava apenas o redirect que o navegador joga fora,
  deixando o histórico criptografado da sessão anterior
  descriptografável. E uma prop `once` agora é pulada somente numa visita
  Inertia completa: um `router.reload({ only: ['stats'] })` explícito a
  resolve de novo em vez de não retornar nada.

- **O transporte SES agora envia headers de mensagem customizados.**
  `Mail::to(..).header("List-Unsubscribe", ...)` e `Mailable::headers()` eram
  descartados silenciosamente sob `MAIL_DRIVER=ses`: o corpo de solicitação
  `Content.Simple` não tinha campo `Headers` e o builder de MIME bruto nunca
  lia `OutgoingMessage::headers`, ainda que todo outro transporte os encaminhe.
  Os dois caminhos do SES agora os carregam - `Headers` como a lista
  `{Name, Value}` do SES v2, MIME bruto como linhas de header reais - para que
  links de descadastro, headers de threading e dicas de roteamento sobrevivam a
  uma troca de driver. Nomes de header são validados de antemão nos dois
  caminhos - CR, LF e NUL (os bytes de injeção, como o transporte do Mailgun já
  recusa) e qualquer coisa que não seja um nome de campo RFC 5322 válido
  (espaços, dois-pontos, não ASCII) - então anexar um arquivo nunca muda se uma
  mensagem é aceita.

### Corrigido


- **Falhas de validação aninhadas agora chegam ao corpo do 422.** Falhas
  de `#[validate(nested)]` em um struct aninhado ou em um elemento de um
  `Vec<T>` validado eram descartadas entre o validador e a resposta: a
  solicitação era corretamente rejeitada com 422, mas o mapa `errors`
  voltava vazio, então nenhuma mensagem era renderizada e o cliente não
  conseguia dizer qual campo estava errado. Falhas aninhadas agora são
  achatadas na notação pontilhada do Laravel - `address.street`,
  `items.1.name`, `order.items.2.sku` - ao lado das de nível superior.

- **O `url` do objeto de página do Inertia mantém a query string.**
  `page.url` era apenas o path da solicitação, então o cliente registrava
  `/users` para uma visita a `/users?page=2&sort=name`. Toda navegação
  para trás/frente e todo `router.reload()` então reproduziam a página
  sem seu cursor de paginação, ordenação, ou filtros. Agora é path mais
  query - a mesma derivação que o `InertiaVersionMiddleware` já usava
  para `X-Inertia-Location`, então por padrão os dois concordam byte a
  byte. O novo `InertiaConfig::url_resolver(...)` sobrescreve como o
  *objeto de página* nomeia a página (o `Inertia::resolveUrlUsing` do
  Laravel); o bounce de versão continua nomeando a URL que chegou, porque
  é essa a URL que o navegador precisa buscar.

- **`Inertia::install` agora aplica sua config a toda resposta.** A
  config entregue a `Inertia::install` era lida em busca de três campos e
  depois descartada, então todo `InertiaResponse` construído sem um
  `.with_config(...)` explícito era renderizado a partir de
  `InertiaConfig::default()`. Um app com scaffold criado com
  `--frontend react` servia o ponto de entrada do Svelte e nenhum
  preâmbulo de refresh do React, a menos que `SUPRNOVA_FRONTEND`
  estivesse definido no ambiente; o SSR habilitado na config nunca
  alcançava uma resposta; e a versão de asset do objeto de página vinha
  de uma config diferente da do resolver do middleware de versão. A
  config instalada agora é retida no registro Inertia do contêiner, e é
  dela que `InertiaResponse::new` parte. O `.with_config(...)` por
  resposta ainda sobrescreve, apps que nunca chamam `Inertia::install`
  ficam inalterados, e um install que falhou (falha fechada) não retém
  nada. Como efeito colateral, o manifesto de produção do Vite agora é
  interpretado uma vez por processo em vez de uma vez por resposta.

- **Apps com scaffold agora instalam os middlewares do protocolo
  Inertia.** O `bootstrap.rs` escrito por `suprnova new` registrava os
  middlewares de sessão, locale, CSRF e include, mas nunca chamava
  `Inertia::install`, então um app gerado não tinha nem
  `InertiaVersionMiddleware` nem `Inertia303Middleware`: um navegador
  ainda rodando o bundle anterior nunca era avisado para recarregar
  depois de um deploy, e um `PUT`/`PATCH`/`DELETE` que redirecionava
  continuava num `302` que o cliente podia seguir com o verbo original. A
  chamada agora fica depois do `SessionMiddleware` - onde o refazer do
  flash de sessão do middleware de versão funciona - com uma constante
  nomeada `INERTIA_VERSION` para incrementar quando os assets mudarem, e
  ela fixa o frontend com o qual o projeto foi gerado
  (`.frontend(Frontend::React)` para `--frontend react`), para que o
  shell HTML carregue o ponto de entrada do Vite daquele framework em vez
  de recair para o do Svelte. O `.env` gerado agora define
  `SUPRNOVA_FRONTEND` para casar. O starter `--api` está inalterado; ele
  não tem frontend.

- **`Queue::push_unique` não relata mais um job enfileirado como
  pulado.** O valor de retorno era calculado com
  `matches!(outcome, Idempotent::Fresh(()))`, que dobrava
  `Idempotent::FreshUnfenced` em `false` - o desfecho em que o envelope
  *foi* enviado, mas o lease de dedupe foi perdido no meio do push.
  Chamadores que ramificavam sobre esse booleano eram informados de que
  um job prestes a rodar tinha sido suprimido como duplicata. Os três
  desfechos agora são casados exaustivamente: um lease perdido retorna
  `true` com um `warn` nomeando o job e sua chave única, e apenas uma
  duplicata real retorna `false`. `push_unique_later` e `later_unique`
  compartilham o caminho e são corrigidos junto.

### Alterado


- **A linha de base de paridade passou para o Laravel 13.25.0.** As notas
  de lançamento 13.23.0, 13.24.0 e 13.25.0 foram rastreadas item a item
  contra a própria superfície do framework. Tudo o que alcançou um
  caminho de código do Suprnova ou está corrigido nesta versão ou tem uma
  linha em [`manual/parity.md`](parity.md) marcada como `not yet`
  ou `by design no`.

### Atualizando

Duas mudanças podem alterar um app em execução sem nenhuma mudança de
código do seu lado.

- **Configurações na config que você passa para `Inertia::install` agora
  fazem efeito.** Elas eram lidas em busca de três campos e descartadas.
  Se a sua config de install define `.ssr(...)`, o SSR agora está ligado:
  inicie o worker (`suprnova ssr:start`) antes de implantar, ou remova a
  chamada `.ssr(...)`. `.entry_point`, `.assets_base_url`,
  `.default_title` e `.encrypt_history(...)` definidos ali também
  alcançam a página agora.

- **`rules::Url` rejeita mais.** Valores que antes passavam e não passam
  mais: qualquer esquema fora da allowlist do Laravel, `javascript:` e
  `vbscript:` entre eles; `mailto:`, `data:` e `tel:`, que estão na
  allowlist mas não carregam host após `://`; e `scheme://` com host
  vazio, como `file:///path`. Se a sua intenção era aceitar um esquema,
  nomeie-o: `Url::protocols(&["myapp"])`.

## 1.2.3 - 2026-08-16

### Corrigido

- **Os casts de data e hora agora leem o texto `CURRENT_TIMESTAMP` nativo do
  banco de dados.** `AsDateTime`, `AsImmutableDateTime` e
  `AsOptionalDateTime` continuam escrevendo RFC-3339 canônico, mas as leituras
  também aceitam texto do PostgreSQL com fuso e valores do SQLite/MySQL sem
  fuso. Valores sem fuso são interpretados como UTC.

## 1.2.2 - 2026-08-14

### Corrigido

- **Valores anuláveis não textuais agora funcionam em todas as escritas
  baseadas em attributes no PostgreSQL.** `Builder::update_all` e
  `Builder::upsert` tipados, `DB::table().insert/update` sem model e extras de
  pivot many-to-many emitem nulls JSON explícitos como `NULL` SQL, continuando
  a vincular todos os valores não nulos. Isso preserva o tipo da coluna de
  destino em vez de enviar um parâmetro null tipado como texto que o PostgreSQL
  rejeita para colunas bigint, integer, boolean, timestamp e outras não
  textuais. Upserts de várias linhas agora também rejeitam colunas ausentes ou
  extras em vez de converter silenciosamente uma linha malformada em null.
  Timestamps automáticos de pivots many-to-many são vinculados como datetimes
  UTC tipados em vez de texto.

### Segurança

- **O gate de lançamento agora distingue metadados dormentes do lockfile de
  dependências compiladas em todo o workspace.** O Cargo registra a dependência
  opcional de compatibilidade rkyv 0.7 não utilizada do rust_decimal em
  `Cargo.lock`; o gate agora comprova que nem o rkyv nem seu crate de derive são
  alcançáveis por qualquer membro do workspace, feature, target ou aresta de
  dependência. A exceção correspondente do RustSec é atribuída, expira em
  2026-11-14 e deve ser removida quando o rust_decimal deixar de registrar essa
  dependência opcional legada.

## 1.2.1 - 2026-08-09

### Alterado

- **O Suprnova mudou da organização entrepeneur4lyf para `eas4ai` no
  GitHub.** URLs do
  repositório em metadados de pacotes, documentação, exemplos de dependências e
  templates de scaffold agora usam `github.com/eas4ai`. Projetos novos também
  usam o e-mail de autor monitorado `shawn@eas4ai.com`. Esta versão não mudou o
  comportamento em runtime.

## 1.2.0 - 2026-08-05

### Adicionado

- **O manual é distribuído em sete idiomas.** `manual/es/`, `manual/fr/`,
  `manual/de/`, `manual/pt-BR/`, `manual/ja/` e `manual/zh-Hans/` trazem
  cada um o manual completo de 104 capítulos - cada capítulo, o sumário
  e este registro de mudanças - traduzido a partir da fonte em inglês. O
  inglês continua canônico: a estrutura dos capítulos, os blocos de
  código, os identificadores, os comandos de CLI e as variáveis de
  ambiente são mantidos byte a byte idênticos à fonte, então um capítulo
  traduzido nunca pode discordar do inglês sobre o que o framework faz -
  apenas dizê-lo no idioma do leitor.

  As traduções foram produzidas e revisadas para o suprnova.app, que
  renderiza este manual como o seu `/docs`. Cada seção carrega lá um
  registro de revisão: os veredictos são registrados contra hashes de
  conteúdo tanto do inglês quanto da tradução, dois revisores
  independentes precisam aprovar os bytes exatos para que uma seção
  conte como aprovada, e glossários por idioma fixam as decisões de
  terminologia (quais termos ficam em inglês, quais tomam a palavra
  nativa, e por quê). Correções são bem-vindas em qualquer um dos dois
  repositórios - uma correção aqui chega ao site na sua próxima
  sincronização.

## 1.1.0 - 2026-08-02

### Adicionado

- **Cadeias de fallback por locale.** `LocalizationConfig` ganha
  `parents` (`APP_LOCALE_PARENTS`, pares `child=parent` separados por
  vírgula, ou o construtor encadeável `.parent(child, parent)`): um
  locale pode herdar de um locale irmão configurado antes de recuar
  ainda mais para o `fallback_locale` global - `pt-PT` a partir de
  `pt-BR`, `en-AU` a partir de `en-GB`, e assim por diante,
  transitivamente. `Lang::get`/`try_get`/`get_with`/`try_get_with`/`has`
  percorrem a cadeia inteira, começando pelo locale atual, então isso
  funciona para qualquer driver `Translator`, não só o embutido. Um par
  malformado, um locale inválido, um filho nomeado duas vezes, ou um
  ciclo (incluindo um locale se nomeando como seu próprio pai) falha de
  forma explícita no carregamento da config, em vez de degradar em
  tempo de solicitação.

  Os catálogos servidos já chegam achatados ao longo da cadeia,
  calculados com antecedência: o `FluentTranslator` agora constrói o
  catálogo `/_suprnova/lang/<locale>.ftl` de cada locale como um fold -
  o catálogo do framework embutido na base para locales `en`/`en-*`,
  depois a cadeia de pais configurada do locale, depois seus próprios
  arquivos `*.ftl` - de forma que um locale encadeado continua sendo um
  único arquivo autocontido que o navegador busca uma vez, sem que o
  cliente precise ter consciência da cadeia. O achatamento cobre só os
  pais configurados; o `fallback_locale` terminal continua sendo um
  fallback no nível da facade `Lang`, e não fica embutido nos bytes
  servidos.

  Isso torna práticos os catálogos em estilo delta: um diretório
  `lang/pt-PT/` pode conter só o punhado de strings que realmente
  diferem de `lang/pt-BR/`, em vez de um catálogo duplicado completo. O
  merge que torna isso possível funciona no nível da AST do Fluent - o
  valor do filho substitui o do pai, os atributos são mesclados por
  nome (um override que não menciona um atributo deixa de perdê-lo),
  expressões select são substituídas por inteiro (as categorias
  plurais do CLDR dependem do locale, então mesclar variante por
  variante não seria coerente), e entradas exclusivas do filho são
  anexadas. Veja a nova seção "Fallback chains" de
  `manual/localization.md` para o contrato completo.

### Alterado

- **`LocalizationConfig` ganhou o campo `parents`.** `from_env()` e o
  construtor não são afetados; um construtor de struct literal (testes
  que constroem um `LocalizationConfig` à mão) precisa de mais um
  campo.
- **O texto de catálogo servido agora é normalizado pelo serializer
  para todo locale**, e o merge multi-arquivo intra-locale (vários
  arquivos `.ftl` em um mesmo diretório de locale) agora passa pelo
  mesmo merge no nível de AST usado nas cadeias de pais, em vez da
  simples sobrescrita de bundle. As traduções resolvidas ficam
  inalteradas, exceto pelas duas melhorias estritas abaixo; os bytes
  subjacentes giram de qualquer forma - `ETag`/`?v=<hash>` gira uma vez
  na atualização. As melhorias: um override não descarta mais
  silenciosamente os atributos que não menciona, e um override que
  contém somente atributos não elimina mais o valor próprio da
  mensagem (anteriormente um erro ou uma resolução de fallback; agora
  ele resolve para o valor do override anterior).

## 1.0.0 - 2026-08-02

### Adicionado

- **Localização.** Catálogos de mensagens em `lang/<locale>/*.ftl`
  ([Fluent](https://projectfluent.org)), uma facade `Lang` com a macro
  `__!("key", name: value)`, detecção de locale por solicitação
  (`LocaleMiddleware`: sessão → cookie → `Accept-Language` →
  `APP_LOCALE`), e formatação sensível a locale para números, moeda,
  datas, horários, listas e tempos relativos sobre o ICU4X.
  `manual/localization.md` é o capítulo.

  As regras de validação embutidas param de fixar o inglês no código.
  Cada uma retorna uma mensagem com chave (`validation-min` mais seus
  argumentos e um fallback em inglês), traduzida uma única vez na
  fronteira de serialização - assim um app em espanhol recebe erros de
  validação em espanhol só ao incluir `lang/es/validation.ftl`, sem
  envolver a regra e sem um fork das mensagens do framework. Os nomes
  de campo são humanizados por uma busca `field-<name>`. `Rule::passes`
  (e `ContextualRule` / `AsyncRule`) agora retornam
  `Result<(), ValidationMessage>`; o corpo `Err("…".into())` de uma
  regra personalizada ainda compila e ainda renderiza literalmente, mas
  a assinatura no seu `impl` precisa do novo tipo.

  O navegador recebe os mesmos bytes que o servidor resolveu: o
  catálogo mesclado é servido em `/_suprnova/lang/<locale>.ftl` com um
  ETag e uma forma imutável `?v=<hash>`, os três starter kits o
  interpretam com `@fluent/bundle`, e `suprnova generate-types` emite
  uma union `MessageKey` para que renomear uma mensagem aponte o
  compilador TypeScript para cada call site.

  Fluent em vez de arrays PHP no estilo Laravel porque um único formato
  precisa servir tanto o servidor quanto o navegador, e porque as
  categorias plurais do CLDR são o que acerta russo, polonês e árabe -
  os intervalos inteiros do `trans_choice` não conseguem, e é por isso
  que não há `trans_choice` aqui. Atrás de uma feature `localization`
  ativada por padrão; `--no-default-features` ainda compila e ainda
  valida, usando os fallbacks em inglês embutidos.

- **`IntoInertiaScroll` para `Paginator`.** A trait estava implementada
  para `LengthAwarePaginator` e `CursorPaginator`, mas não para o
  paginador simples, então resultados de `simple_paginate` não
  conseguiam alimentar `Inertia::paginate` de forma alguma - apesar de
  a própria documentação do módulo `simple.rs` apontar para ele como o
  caminho de geração de URL. Isso deixava coleções Inertia paginadas
  por offset com a escolha entre um `COUNT(*)` por solicitação e
  montar à mão os metadados de scroll. `next_page` vem da sonda de
  overflow do `LIMIT n+1`, em vez de uma última página calculada, já
  que não há total a partir do qual calculá-la.

### Corrigido

- **`suprnova generate-types` emitia um arquivo diferente a cada
  execução.** A ordenação topológica semeava sua fila de trabalho
  iterando um `HashMap`, e o Rust randomiza a ordem de iteração do
  hash por processo, então execuções consecutivas ordenavam as mesmas
  interfaces de formas diferentes. A saída é um artefato versionado,
  então toda execução produzia um diff - e um arquivo gerado que muda
  sem motivo é um arquivo que as pessoas param de regenerar, depois do
  que ele silenciosamente para de descrever o Rust que afirma
  descrever. A varredura de diretório também passou a ser ordenada,
  então a saída também não depende mais da ordem do sistema de
  arquivos. Duas execuções da mesma fonte agora são idênticas byte a
  byte.

- **`topological_sort` fazia o oposto do seu comentário de
  documentação**, emitindo dependentes antes de dependências.
  Inofensivo - uma interface TypeScript pode referenciar uma declarada
  mais adiante no mesmo arquivo - então o comentário foi corrigido em
  vez da ordem, o que teria reembaralhado um arquivo versionado sem
  nenhum benefício.

## 0.9.1 - 2026-08-01

Três defeitos, todos encontrados rodando o app de dogfood sob um
harness containerizado, em vez de lendo o código. Cada um deles é
invisível para uma suíte de testes que nunca para um processo do jeito
que produção para.

Eles se compõem numa ordem específica: um deploy rolling manda SIGKILL
em um worker no meio de um job (o primeiro), e esse job então toma um
caminho de reclaim que nunca contou a tentativa (o segundo).

### Corrigido

- **`schedule:work`, `queue:work` e `workflow:work` ignoravam
  SIGTERM.** Cada um selecionava só sobre `tokio::signal::ctrl_c()`,
  que instala um handler de SIGINT - então SIGTERM não tinha handler
  algum em lugar nenhum do processo, e SIGTERM é o que `docker stop`,
  Coolify, systemd e Kubernetes enviam. Os três já tinham um drain
  cuidadoso e limitado atrás daquele `select!`; nada disso jamais
  havia executado sob um supervisor. Medido antes da correção: um
  `docker stop` num contêiner `queue:work` queimava toda sua janela de
  graça de 40s e saía com código 137, com o job em voo destruído. Como
  PID 1 - que é o que um contêiner executa -, o kernel descarta um
  SIGTERM não tratado diretamente, então o processo não morria mal;
  ele simplesmente não morria até o SIGKILL. `Server::run` já tratava
  os dois sinais corretamente e seu listener agora é compartilhado, o
  que também fecha uma janela de sinal perdido no loop do agendador.

- **Um job que matava seu worker nunca podia ser dead-lettered.** Um
  job cujo *handler* falha recebe nack e tem sua tentativa contada,
  então ele vai para dead-letter depois de `max_tries`. Um job que
  *mata seu worker* - OOM, abort, segfault, ou o SIGKILL acima - não
  liquida nada; sua reserva simplesmente expira, e todo driver
  costumava reentregá-lo byte-idêntico. Um job assim é imortal: mata
  cada worker que o reivindica, volta inalterado, e mata o próximo,
  por quanto tempo qualquer coisa reiniciar workers. Os três drivers
  agora cobram a tentativa no momento em que descobrem que um worker
  morreu, porque trocar `QUEUE_DRIVER` não pode mudar se um job
  envenenado pode ser parado. `attempts` agora significa "entregas a
  um worker" em vez de "falhas de handler" - documentado em
  `manual/queues.md`, porque um worker perdido por razões alheias
  também queima uma tentativa.

- **…e o job esgotado agora vai para dead-letter antes de ser
  despachado.** Contar a tentativa era necessário, mas não suficiente.
  Toda decisão de dead-letter vivia no caminho de liquidação do
  worker, que assume que o handler retorna - então ela nunca rodava
  exatamente para os jobs que não conseguiam retornar. Só com a
  correção do driver o contador subia (medido: 0 → 1 → 2 ao longo de
  três workers mortos) e nada agia sobre isso. O orçamento agora é
  gasto antes do handler rodar. Descoberto só ao rodar de novo o
  experimento no contêiner, depois que a primeira correção parecia
  correta.

- **Os daemons não tinham subscriber de tracing.** `serve` recebe um
  de `init_telemetry`; `queue:work`, `schedule:work`, `schedule:run` e
  `workflow:work` passam por um caminho de boot diferente e não
  recebiam nenhum, então toda linha `tracing::` que emitiam ia parar
  em lugar nenhum, e `LOG_LEVEL` era inerte para eles. Isso é a maior
  parte do que eles têm a dizer - um worker mandando um job para
  dead-letter, um agendador pulando um tick que perdeu, um lock que
  não conseguiu liberar. Num contêiner a única saída visível era o
  banner de inicialização, e o processo parecia ocioso enquanto fazia
  tudo isso. Dois dos defeitos deste release eram invisíveis até isso
  ser corrigido.

- **Um dead-letter sem um armazenamento de failed-jobs vinculado era
  uma exclusão silenciosa.** O passo de persistência ficava dentro de
  `if let Some(store) = ..`, então sem um store o braço não casava e a
  execução caía direto no ack - mais silencioso que o caminho de falha
  logo acima, que ao menos deixa a reserva intacta. Um store ausente
  era tratado como mais bem-sucedido que um quebrado. Agora ele
  registra o envelope inteiro em ERROR, porque é isso que
  `queue:retry` reempurra: a diferença entre trabalho recuperável à
  mão e trabalho que deixou de existir.

- **`QUEUE_DRIVER=database` agora vincula um armazenamento de
  failed-jobs.** `failed_jobs` faz parte do contrato desse driver -
  `queue:retry` o lê e `Queue::retry_failed` não funciona sem ele -
  mas `bootstrap_from_env` conectava o driver e deixava o store sem
  definir, então uma fila apoiada em banco de dados mandava para
  dead-letter no nada, a menos que o app vinculasse um à mão.
  Configurável via `QUEUE_FAILED_DB_TABLE`. Só para este driver:
  `memory` é efêmero por construção e `redis` não tem tabela para
  escrever.

- **A latência de reclaim do Redis agora acompanha
  `--visibility-timeout`.** A flag define o limiar de idle do
  XAUTOCLAIM, mas um clock separado governa a frequência com que um
  consumer olha, e o driver a deixava no padrão de 30s do
  sea-streamer - então `--visibility-timeout 5` na prática significava
  "até 35 segundos". O intervalo agora acompanha o timeout
  configurado, limitado entre 1s e 30s, de forma que um timeout curto
  não pode virar uma tempestade de XAUTOCLAIM, e um longo só pode
  tornar o reclaim mais rápido que antes.

### Adicionado

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** -
  executa uma tarefa agendada exatamente uma vez por tick devido,
  entre réplicas. Sem isso nada elege um líder para um tick: cada
  processo `schedule:work` avalia o schedule independentemente, e três
  réplicas foram medidas executando toda tarefa devida três vezes, a
  cada minuto, sem variância nenhuma. Um job noturno de faturamento em
  três réplicas faturava cada cliente três vezes.

  `without_overlapping()` não cobre isso e não pode: seu lock é
  chaveado na tarefa e liberado quando o handler retorna, então uma
  tarefa rápida o libera antes de uma segunda réplica olhar.
  `on_one_server` chaveia na tarefa *e no tick* e segura o lock além
  do handler, deixando-o expirar por TTL. Os dois se compõem.

  Opt-in, seguindo o Laravel. Diverge do Laravel ao falhar de forma
  fechada: a eleição só é tão compartilhada quanto o cache por trás
  dela, então um boot de produção com `CACHE_DRIVER=memory` e uma
  tarefa de servidor único é recusado, nomeando as tarefas culpadas,
  com `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` para
  deployments que genuinamente rodam um único scheduler.

### Alterado

- `manual/deployment.md` não diz mais "rode exatamente um processo
  `schedule:work`" como única opção, e ganha uma seção **Stopping
  cleanly** cobrindo as janelas de drain por subsistema, como
  dimensionar a graça de terminação de uma plataforma acima delas, e
  por que PID 1 torna um handler de sinal ausente pior do que parece.

## 0.9.0 - 2026-07-31

### Segurança

- **A emissão de auth só podia ser limitada por caller, nunca por
  destinatário.** Um limite chaveado por endereço responde "um cliente
  está barulhento?"; não consegue responder "uma caixa de entrada está
  sendo inundada?". Um atacante espalhado por uma botnet ou por um
  único `/64` IPv6 permanecia abaixo de todo orçamento por IP enquanto
  enchia a caixa de entrada de uma vítima com e-mail de redefinição de
  senha, e nada no framework conseguia expressar o limite que teria
  impedido isso - uma key function conseguia ler o path, os
  cabeçalhos e a query string, mas não um corpo form-encoded, então o
  endereço era invisível exatamente na rota que o carrega.

  `identity_key` chaveia um bucket na conta sendo afetada. Ela lê a
  query string primeiro e depois um corpo de formulário bufferizado,
  então uma única key function cobre as duas formas; o valor é
  aparado e colocado em minúsculas, porque `Alice@Example.com` chega
  na mesma caixa de entrada que `alice@example.com`, e um limite
  contornável segurando o shift não é um limite; e é hasheado, porque
  um backend de rate limit é frequentemente um Redis compartilhado com
  controle de acesso mais fraco que o do banco de dados primário.

  Dois novos construtores de middleware dão suporte a isso.
  `key_reads_body(cap)` bufferiza o corpo antes de chavear - opt-in,
  porque bufferizar é trabalho que um caller não autenticado consegue
  te obrigar a fazer, e um corpo acima do cap é recusado com 413 em
  vez de passado adiante sem chave. `only_when(pred)` pula um limiter
  inteiramente para solicitações sobre as quais ele não tem nada a
  dizer, o que é o que impede um orçamento por destinatário empilhado
  de silenciosamente virar o limite vinculante em rotas que não
  nomeiam destinatário nenhum.

  O app de dogfood agora empilha os dois no seu grupo de emissão: 10
  a cada 5 minutos por endereço, 3 a cada 15 minutos por destinatário.

Uma revisão dos caminhos de sessão, senha, OAuth e passkey do Torii
revelou oito defeitos, todos corrigidos no fork fixado
(`suprnova-torii-rs` `968b0be`).

- **Sessões expiradas podiam ser renovadas de volta à vida.** O
  `refresh` do repositório de sessão do SeaORM não tinha predicado de
  expiração e estendia `expires_at` incondicionalmente, e
  `OpaqueSessionProvider::refresh_session` pulava a checagem
  `is_expired()` que `get_session` faz. Um token mantido além de sua
  expiração podia ser renovado indefinidamente. Corrigido nas duas
  camadas. Não alcançável pela própria superfície do Suprnova - nem o
  `Torii` nem o framework expõem renovação de sessão - mas é API
  pública dos dois crates.
- **O formulário de login vazava quais contas existem, por timing.** A
  autenticação retornava assim que o e-mail não batia, pulando o
  Argon2 inteiramente: medido em 54µs para um endereço desconhecido
  contra 719ms para uma senha errada, uma diferença de ~13.000x
  legível pela rede. Os dois caminhos de falha agora verificam contra
  um hash fictício, então custam o mesmo. Este *era* alcançável pelo
  login por senha do Suprnova.
- **A claim `iss` do JWT era escrita, mas nunca verificada.** A
  fixação de algoritmo já estava correta - `alg: none` e a confusão
  HS/RS nunca foram possíveis - mas o issuer era decoração, então dois
  serviços compartilhando uma chave de assinatura aceitariam as
  sessões um do outro. Agora aplicado quando um issuer é configurado.
- **Um verificador PKCE de uso único podia ser reivindicado duas
  vezes.** O consumo era uma leitura seguida de uma exclusão, então
  dois callbacks OAuth para o mesmo `csrf_state` podiam ambos ler
  antes de qualquer exclusão acontecer. Agora reivindicado em uma
  única operação - `DELETE ... RETURNING` no Postgres, uma exclusão
  por chave primária cuja contagem de linhas afetadas escolhe o
  vencedor no SeaORM.
- **Sessões expiradas eram listadas como ativas.** `find_by_user_id`
  não tinha filtro de expiração, e linhas expiradas sobrevivem até a
  limpeza rodar, então uma tela de "dispositivos em que você está
  conectado" oferecia aos usuários sessões mortas para revogar,
  sem dizer nada sobre a que estava viva.
- **Uma busca de passkey se chamava `authenticate`.** O
  `PasskeyService::authenticate_credential` do Torii recebia um ID de
  credencial e retornava o usuário dono, e `PasskeyAuth::authenticate`
  cunhava uma sessão a partir disso. O Torii armazena passkeys - não
  carrega dependência de WebAuthn nenhuma e não consegue verificar uma
  assertion, então a única coisa que essas chamadas provavam era que o
  caller conhecia um ID de credencial: um valor que o navegador envia
  em claro e que `allowCredentials` entrega a qualquer um que consiga
  iniciar uma cerimônia. Renomeado para `find_user_by_credential` e
  `create_session_for_verified_credential`, ambos documentando que a
  verificação é trabalho do caller. Não alcançável através do
  Suprnova, que dirige o próprio `webauthn-rs` (veja
  `torii_integration::passkey`) e só alcança o Torii para
  armazenamento de credenciais.
- **Um desafio WebAuthn podia sofrer replay durante todo o seu TTL.**
  Nenhum dos dois backends consumia um desafio na leitura, e o
  `get_challenge` do SeaORM também ignorava `expires_at` por
  completo, retornando desafios expirados como se estivessem vivos.
  Leituras agora excluem linhas expiradas nos dois backends, e um novo
  `take_challenge` reivindica um exatamente uma vez - a mesma forma de
  "a exclusão decide o vencedor" da correção do PKCE.

### Mudanças incompatíveis

- **Azure Blob Storage e Google Cloud Storage se mudaram para trás
  das novas features `filesystem-azure` e `filesystem-gcs`.**
  `Storage::register_azblob`, `register_azblob_with`, `register_gcs`,
  `register_gcs_with`, `AzBlobConfig` e `GcsConfig` não existem mais a
  menos que você ative a feature correspondente. Se você usa qualquer
  um dos dois backends, adicione-a à sua dependência:

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  Você recebe um erro de compilação nomeando o item ausente, não uma
  falha em tempo de execução.

  Os dois crates de serviço do opendal puxam `rsa`, que carrega o
  RUSTSEC-2023-0071 (o ataque de timing Marvin) sem release corrigido
  upstream. Eram os únicos crates ativando `reqsign-core/jwt`, a
  feature atrás da qual está o `rsa` opcional do `reqsign-core`, então
  colocá-los atrás de gate corta os três caminhos do opendal até ele
  de uma vez. `rsa` agora é *evitável*: `--no-default-features
  --features filesystem,database-postgres` resolve sem ele e ainda
  tem o subsistema de storage. Antes, nenhuma combinação de features
  conseguia se livrar dele mantendo o storage de alguma forma.

  Um build padrão de fábrica ainda carrega `rsa` - `database-mysql` é
  uma feature padrão e `sqlx-mysql 0.8.6` depende dele de forma não
  opcional - então a exceção de auditoria continua aberta. O S3
  deliberadamente **não** fica atrás de gate: `reqsign-aws-v4` usa
  `reqsign-core` sem `jwt`, então o driver S3 nunca contribuiu com um
  caminho até ele, e colocá-lo atrás de gate quebraria o backend de
  nuvem mais usado sem remover nada.

### Adicionado

- **`suprnova --version`**, com `-v` além do `-V` padrão do clap.
  Perguntar a versão de uma CLI com a flag que toda outra CLI usa não
  devia imprimir um erro de uso.

### Corrigido

- **Duas operações do Redis não tinham limite superior.** O flush de
  tag do cache lia o conjunto de membros inteiro de uma tag com
  `SMEMBERS` e excluía chave por chave, então uma tag com uma
  associação grande travava a conexão, e uma escrita concorrente podia
  se perder entre a leitura e a exclusão; tags agora são baseadas em
  geração, liberadas atomicamente, e varridas com um `SSCAN` limitado.
  O passo de promoção da fila atrasada movia todo job devido em um
  único `ZRANGEBYSCORE` sem limite, então um backlog que vencia junto
  produzia um único script enorme; agora ele promove em lotes.
- **Dois drains de shutdown esperavam para sempre.** `schedule:work`
  no Ctrl-C e o worker de workflow após cancelamento aguardavam cada
  um toda tarefa em voo sem prazo, então uma tarefa que nunca
  retornava mantinha o processo aberto até o `SIGKILL` - um operador
  vê um daemon que "não para". Os dois agora esperam uma graça
  limitada, depois abortam o que resta e reportam a contagem.
- **A varredura de fixação de versão do release só reconhecia uma das
  duas sintaxes de fixação**, então todo arquivo carregando uma linha
  `cargo install --tag vX.Y.Z` e nenhum trecho de dependência nunca
  era descoberto. `suprnova-cli/README.md` vinha dizendo aos leitores
  para instalar a v0.6.0 havia três releases; `manual/cli.md` e
  `manual/cli-new.md` estavam parados na v0.7.2; `manual/installation.md`
  carregava as duas formas e tinha uma atualizada enquanto a outra
  congelava. A descoberta e a reescrita agora leem de uma única tabela
  de padrões, e as regras de um arquivo derivam do seu conteúdo.
- **`cargo doc` falhava para qualquer build com `filesystem`, mas sem
  `testing`** - sete links intra-doc de `Storage::fake` não
  conseguiam resolver, e `lib.rs` nega links quebrados. `testing` é
  uma feature padrão, então nenhum passo de gate jamais tinha
  construído essa combinação; `check-feature-matrix.sh` agora faz
  isso.
- **As migrações do Torii não podiam ser replayadas sobre seu próprio
  schema**, então um banco de dados que o mantinha sem a tabela de
  rastreamento `torii_migrations` - restaurado de um dump que a pulou,
  ou migrado à mão - não podia ser trazido sob gestão. Todo
  `Table::create()` carregava `.if_not_exists()`; nenhuma das 19
  chamadas `Index::create()` carregava, nem o alter `ADD COLUMN
  locked_at`, então o replay passava pelas tabelas e morria no
  primeiro `CREATE INDEX`. Corrigido no fork fixado
  (`suprnova-torii-rs` `a0f956d`) via `has_index` / `has_column` em
  vez de `IF NOT EXISTS`, que o sea-query silenciosamente descarta
  para MySQL - a correção sintática teria deixado quebrado um build
  com as features padrão.
- **Uma migração do Torii que falhava abortava o processo em vez de
  retornar um erro.** `SeaORMStorage::migrate` desembrulhava
  (unwrap) o migrador e retornava `Ok(())` incondicionalmente,
  então o mapeamento que `init_torii` fazia da falha para um
  `FrameworkError` era código inalcançável.
- **A própria tabela `users` de um app suprimia silenciosamente a do
  Torii**, porque `.if_not_exists()` não consegue distinguir "já é
  minha" de "já é de outra pessoa". A migração reportava sucesso e a
  autenticação falhava depois numa coluna ausente - a razão pela qual
  o starter `--api` nomeia sua tabela `app_users`. A migração do
  Torii agora avisa, no momento da migração, quando uma tabela
  `users` existente não tem colunas que ela exige, nomeando as
  colunas e o remédio. Continua sendo um aviso, não uma falha dura,
  para que deployments existentes continuem inicializando.
- **Os guias de deployment do Railway e da DigitalOcean apontavam o
  health check da plataforma para um path que podia sondar o
  Postgres.** As duas plataformas reiniciam o contêiner quando essa
  checagem falha, então seguir o conselho transformava uma soluço de
  banco de dados num loop de reinício em toda réplica. As duas agora
  usam `/_suprnova/health/live`, com o banco de dados sondado à mão
  pelo console. Os paths legados ainda resolvem; nada já implantado
  precisa mudar.

## 0.8.0 - 2026-07-30

Remediação de uma auditoria externa de red team. A auditoria retornou 19
achados P1 e um veredito NO-GO para o 1.0; este release fecha **os
dezenove**, mais vários defeitos encontrados enquanto os corrigia que a
auditoria não tinha nomeado.

Várias correções deliberadamente transformam uma configuração incorreta
silenciosa num boot recusado. Leia **Atualizando** antes de fazer
deploy - um app de produção que vinha rodando feliz pode não iniciar.

### Atualizando

Três configurações que costumavam inicializar com um aviso (ou em
silêncio) agora falham de forma fechada em produção. Cada erro nomeia
a variável que o destrava, e cada uma tem uma sobrescrita explícita
para o deployment onde o risco genuinamente não existe.

- **Um driver de mail que não entrega.** `MAIL_DRIVER` sem definir,
  `log`, `memory`, ou um valor não reconhecido, todos resolviam para
  um transporte que renderiza o mail e o descarta - então
  redefinições de senha reportavam sucesso enquanto nada era enviado.
  Sobrescrita: `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
- **SMTP em texto claro.** Três das quatro combinações de credenciais
  caíam num transporte não criptografado, e o caso com as duas
  ausentes registrava um aviso e enviava assim mesmo. Sobrescrita:
  `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`.
- **O rate limiter em memória.** Seus buckets vivem no heap de um
  único processo, então atrás de N réplicas todo quota é na verdade
  N× e cada deploy os reseta. Aponte `RATE_LIMIT_DRIVER` para
  `redis`, ou defina `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true` se
  você genuinamente roda um único processo. Um valor de driver *não
  reconhecido* falha pelo mesmo motivo, porque caía de volta para
  memory - `RATE_LIMIT_DRIVER=Redis`, com maiúscula, é o caso com
  mais chance de chegar a produção porque parece configurado.

Desenvolvimento, testes e staging ficam inalterados nos três casos.
Staging deliberadamente não passa por esse gate: falhar
implacavelmente nele empurra os times a definir a sobrescrita
globalmente, o que desarma a checagem exatamente onde ela importa.

Duas mudanças de comportamento que não são falhas de boot:

- **`fill` e `first_or_new` rejeitam valores malformados.** Um valor
  que não conseguia decodificar para o tipo do seu campo costumava
  virar o `Default` daquele campo e retornar `Ok` -
  `fill(attrs!{ age: "abc" })` definia `age = 0` e reportava sucesso.
  Agora retorna um `ValidationError` nomeando o campo, e deixa o
  model intocado. Colunas desconhecidas continuam sendo puladas
  silenciosamente (paridade com o Laravel), e o alargamento numérico
  continua funcionando.
- **`/_suprnova/health?db=true` não retorna mais o erro do driver.**
  O detalhe se muda para o log; o corpo mantém `"database": "error"`.
  Builds de debug ainda o incluem. Dashboards que fazem parse de
  `status` / `database` não são afetados.
- **`url::signature_has_not_expired` agora exige uma assinatura
  válida**, e está descontinuada. Costumava responder `true` para uma
  URL forjada - uma assinatura ruim não está "expirada", porque nunca
  teve uma expiração para perder - então qualquer handler que se
  protegia só com ela aceitava forjas. Agora é idêntica a
  `has_valid_signature`. Se você a usava para distinguir *expirada*
  de *inválida* (para renderizar "peça um link novo" em vez de um
  403), mude para `url::signature_verdict`, que retorna os três
  estados. Isso diverge deliberadamente do `URL::signatureHasNotExpired`
  do Laravel.

Duas adições que só pedem algo de você se você optar por entrar:

- **`QueueDriver` ganhou `settle` e `release`**, os dois com
  implementações padrão, então impls de driver existentes continuam
  compilando sem alteração. Implemente `settle` se seu backend
  consegue commitar uma escrita de acompanhamento e uma confirmação
  em uma única transação; implemente `release` se ele consegue
  reenfileirar uma mensagem reservada no lugar.
- **A contabilidade de batch agora pode ser durável.**
  `DatabaseBatchRepository` precisa de duas tabelas novas,
  `job_batches` e `job_batch_settlements` - adicione-as às suas
  migrações, como com `jobs` e `failed_jobs`. O schema está em
  `manual/queues.md`. Nada muda se você permanecer no
  `MemoryBatchRepository`.

### Segurança

- **Slowloris (SEC-07).** O timeout de leitura de header do hyper era
  documentado como 30s, mas estava inerte - ele só arma quando um
  timer é instalado no connection builder, e nenhum era. Um cliente
  podia segurar uma conexão, e uma permissão de
  `SERVER_MAX_CONNECTIONS`, indefinidamente. Agora armado e
  configurável via `SERVER_HEADER_READ_TIMEOUT`.
- **Uploads multipart (SEC-05).** O cap se aplicava aos payloads de
  cada part individual, mas não ao stream bruto, então um corpo podia
  exceder o limite em agregado. Agora limitado no stream.
- **HMAC de webhook com chave vazia (SEC-08).** Os dois adaptadores de
  pagamento aceitavam um secret em branco, que verifica qualquer
  coisa. Recusado nos dois.
- **Parsing de assinatura da Paddle (P2-11).** Um `paddle-signature`
  de tamanho ímpar ou não hexadecimal chegava ao SDK fixado e entrava
  em panic dentro dele. Agora validado primeiro: uma assinatura
  malformada é um 401.
- **Cadastro de passkey e tokens de reset (SEC-01, SEC-02).** Cadastro
  anônimo contra um e-mail existente, cadastro por um não-dono, e
  cadastro pelo dono sem reautenticação recente são cada um recusados
  com status distintos. Um login por senha agora carimba a janela de
  reautenticação.
- **`dev:tls` (SEC-10).** Um projeto podia escolher a CA em que o
  comando confia.
- **Docker Compose gerado (P2-12).** Publicava Postgres e Redis em
  todas as interfaces com credenciais commitadas neste repositório.
  Agora vinculado a loopback com senhas geradas por scaffold, `.env`
  escrito com 0600, e alvos symlinkados recusados.
- **Endpoint de health (P2-01, CI-05).** Ele decidia se consultava o
  banco de dados com `query.contains("db=true")` - um teste de
  substring, então `?nodb=true` também rodava a sonda. Agora
  interpretado corretamente. O 503 não embute mais o erro do driver,
  que nomeava hosts, portas, schemas e versões.
- **Limitação de emissão de credenciais (P2-02).** As quatro rotas de
  emissão de auth no app de referência não carregavam rate limit
  nenhum, e a única rota que carregava chaveava seu bucket no header
  bruto `x-forwarded-for` - que qualquer cliente pode variar a cada
  solicitação para obter um bucket novo. As duas corrigidas; o
  orçamento de emissão é compartilhado entre as quatro rotas, então
  alternar entre elas não o multiplica.
- **Um step de chain redelivered reempurrava seu sucessor sob um id
  novo (DATA-02b, parcial).** A liquidação empurra o próximo elo da
  chain *antes* de dar ack, deliberadamente: dar ack primeiro
  significa que um crash nessa janela perde a chain permanentemente, e
  uma duplicata é recuperável onde uma perda silenciosa não é. Mas o
  envelope do sucessor recebia um `Uuid::new_v4()` novo a cada push,
  então a duplicata produzida por essa troca era indistinguível de um
  step novo legítimo - para o driver, para um outbox, e para o
  handler.

  Esse último é o custo real. O contrato de entrega do framework é
  at-least-once e sua resposta para duplicatas é "handlers precisam
  ser idempotentes" - mas um handler chaveado em `env.id`, o único
  identificador que recebe, não conseguia satisfazer esse contrato
  para um job encadeado, porque a duplicata chegava sob um id novo
  toda vez. O contrato era insatisfazível por construção.

  O id do sucessor agora é um UUIDv5 derivado do de seu predecessor,
  que é estável ao longo das próprias redeliveries desse predecessor.
  Um step redelivered reempurra o id que empurrou antes. Nenhuma
  mudança de schema, nenhum campo novo, nenhuma dependência nova.

  Isso torna a duplicata **detectável**, que é a primitiva que faltava
  no resto do DATA-02b. Não torna o push atômico com o ack (isso
  precisa do outbox), e nada ainda rejeita a duplicata na entrada. Os
  dois continuam em aberto.
- **URLs assinadas verificavam uma URL e executavam outra (SEC-04).**
  A forma canônica colapsava pares de query num map, então uma chave
  repetida mantinha só seu **último** valor - enquanto
  `Request::query_param` retornava o **primeiro**. Um `?user=victim`
  legitimamente assinado podia então ser replayado como
  `?user=attacker&user=victim` com a assinatura original intacta: a
  verificação canonicalizava sobre `victim` e passava, e o handler
  agia sobre `attacker`.

  A forma canônica agora carrega todo par, ordenado por `(key,
  value)`, então a assinatura cobre o multiset exato de parâmetros -
  adicionar, remover ou substituir qualquer valor quebra o HMAC. Um
  `signature` ou `expires` repetido é recusado de imediato, já que
  duas ocorrências de qualquer um dos dois não deixam resposta não
  arbitrária sobre qual delas vale.

  `Request::query_param` agora resolve uma chave repetida para seu
  último valor, casando com `query_params` e `Context::query_param`;
  era o único dos três que discordava, e essa discordância era a
  outra metade do defeito. **Links assinados existentes continuam
  funcionando** - sem chaves repetidas os bytes do payload ficam
  inalterados, o que um teste fixa, porque uma mudança de forma
  canônica que invalidasse silenciosamente todo link de redefinição
  de senha pendente seria pior que o bug.

  Seis testes de regressão, incluindo as duas ordens de ataque, uma
  chave legitimamente repetida que ainda precisa assinar e verificar,
  e a garantia de reordenação. *Não* mudado: `signature_has_not_expired`
  continua reportando uma assinatura forjada como "não expirada".
  Esse é o comportamento do Laravel, foi resolvido deliberadamente
  como uma correção de documentação, e tem seu próprio teste
  fixando-o contra uma "correção" bem-intencionada.
- **RBAC sob Postgres.** Verificado contra um Postgres real, e não só
  contra SQLite.
- **Quatro avisos do RustSec eliminados, não renovados.** O driver do
  Pinecone foi reescrito contra a API REST do Pinecone, derrubando
  `pinecone-sdk 0.1.2` - cujo release mais novo data de 2024-09-06 -
  e com ele `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102` e o
  RUSTSEC-2026-0049 / -0098 / -0099 / -0104. Os quatro foram
  corrigidos upstream em `rustls-webpki >= 0.103.13`, que este
  workspace já resolvia para seus outros usuários de TLS; um único
  crate abandonado prendia a árvore na linha vulnerável.
  `.cargo/audit.toml` caiu de cinco exceções para uma. Veja
  **Alterado** para o que isso significa para a API do driver.
- **Exceções de auditoria agora expiram.** Toda entrada em
  `.cargo/audit.toml` carrega um `OWNER` e uma data `EXPIRES`, e
  `scripts/check-audit.sh` falha o gate de release num owner ausente,
  numa data ausente ou não interpretável, ou numa já vencida. `cargo
  audit` não tem noção de uma exceção com prazo, então uma adicionada
  "temporariamente" ficava até alguém reler o arquivo. A entrada
  restante (RUSTSEC-2023-0071, `rsa`, que não tem release corrigido
  nenhum) tem dono e data.
- **Alegações de alcançabilidade são checadas, não simplesmente
  afirmadas.** `scripts/check-feature-matrix.sh` resolve árvores de
  dependência reais e garante que nenhum build - incluindo
  `--all-features`, que é o que `cargo audit` de fato lê - contém
  `pinecone-sdk`, `rustls-webpki 0.102.x` ou `tonic 0.11.x`. Uma
  exceção justificada por um comentário que nada verifica deixa de
  ser verdade na primeira vez que alguém adiciona uma dependência.

### Corrigido

- **Todo release numa fila apoiada em banco de dados era silenciosamente
  um no-op.** `JobOutcome::Released` - um lock `WithoutOverlapping`
  ocupado, um backoff de rate limiter - era implementado como "empurra
  uma cópia, depois dá ack no original". O id do envelope é a chave
  primária da tabela `jobs`, então a cópia colidia com a linha que
  ainda segurava a reserva viva, e o push falhava com
  `UNIQUE constraint failed: jobs.id`. O worker então corretamente se
  recusava a dar ack, então o atraso solicitado nunca era aplicado,
  nenhum evento `JobReleased` disparava, e o job simplesmente ficava
  parado até a expiração de visibilidade redeliverá-lo. Releases agora
  são uma única chamada de driver, feita no lugar.
- **Um dispatch de batch parcial orfanava os jobs que já tinha
  enfileirado (DATA-02).** Quando um `driver.push` falhava no meio do
  loop, `PendingBatch::dispatch` excluía a linha do batch - mas os
  envelopes já na fila continuavam carimbados com aquele id de batch,
  então cada um deles liquidava contra um batch que não existia mais,
  retornando `Err(batch not found)` a cada entrega, para sempre. O
  batch agora é liquidado em vez disso: jobs não despachados são
  registrados como falhas e o batch é cancelado, então os enfileirados
  liquidam normalmente e os callbacks terminais ainda disparam.
- **Nada testava que `url::has_valid_signature` rejeita uma URL
  forjada.** Encontrado ao verificar a correção do SEC-04: a suíte
  inteira do framework passava com a guarda primária de URL assinada
  reescrita para aceitar qualquer assinatura.
- **Um app com scaffold não conseguia migrar seu banco de dados nem
  construir sua imagem (REL-01b).** Nenhum dos dois scaffolds
  declarava `default-run`, então todos os nove wrappers de CLI que
  chamam `cargo run` via shell falhavam num projeto recém-criado. O
  Dockerfile gerado tinha cinco defeitos independentes - um COPY de
  lockfile ausente, `npm ci` sem um lock, um estágio de cache
  stubando um dos dois binários declarados, um build de frontend
  copiado de um path que o vite nunca cria, e um COPY ausente de
  `frontend/src/pages`, que `inertia_response!` valida em tempo de
  compilação. A imagem de um scaffold de fábrica não conseguia
  construir.
- **`docker:init` emitia um único Dockerfile para todo tipo de
  projeto.** Num projeto `--api`, sua primeira instrução, `COPY
  frontend/package.json`, falhava de imediato. Projetos API agora
  recebem um Dockerfile sem frontend.
- **Placeholders SQL (DATA-01).** Renderizados por backend, em vez de
  assumir um único dialeto.
- **Liquidação de fila (DATA-02a, P2-06c).** Follow-ups liquidam antes
  de a reserva receber ack, e um erro de liberação de lock não
  converte mais um job já bem-sucedido num retry.
- **Um batch cancelado disparava `Catch`, nunca `Then`.**
- **`Builder::clone` descartava silenciosamente o plano de eager-load
  (P2-09a).** `User::query().with("posts")` clonado em qualquer
  lugar - paginação, `count()`, qualquer scope que clona - retornava
  linhas sem relações e sem erro.
- **Rosters de presence perdiam membros (P2-08).** O roster tinha seu
  snapshot tirado antes de assinar, então quem entrasse nessa janela
  não aparecia em nenhum dos dois, permanentemente.
- **O Pinecone serializava toda aquisição de índice (P2-14).** O lock
  de escrita era segurado ao longo de dois round trips de rede, e o
  `RwLock` justo do `tokio` fazia um índice frio travar todo índice
  quente.
- **O monitor de tipos descartava rajadas (P2-13).** O debounce de borda
  de subida regenerava no primeiro arquivo de uma rajada e descartava
  o resto sem uma execução final, então o último save nunca fazia
  efeito.
- **`ssr:check` podia travar, e tentava um único endereço (P2-13).** O
  DNS rodava totalmente fora do timeout, e só o primeiro endereço
  resolvido era tentado - então um host com um registro AAAA e sem
  rota IPv6 reportava o worker fora do ar enquanto ele estava
  escutando em v4.
- **`suprnova serve` instalava `cargo-watch` sem fixação (P2-13).**
  Agora com `--locked` e um limite de versão major.
- **O bumper de release reescrevia cinco READMEs e nada mais.** Quatro
  capítulos do manual e um doc comment público fixavam tags que
  release nenhum jamais atualizava - o doc comment estava dois
  releases desatualizado. A descoberta agora substitui a lista mantida
  à mão, e o smoke test faz grep na árvore atualizada
  independentemente, em vez de confiar no próprio passo de verificação
  do bumper.
- **`db:sync` tratava o schema do banco de dados como entrada confiável
  (CLI-01).**
- **`migrate:fresh` fica atrás de gate por `--force` mais uma
  confirmação tipada (CLI-02)**, tanto no binário do app quanto na
  CLI.
- **O driver de mail `log` agora registra a mensagem inteira**, como o
  Laravel faz, e não escreve mais links bearer no log em produção.

### Adicionado

- **Liquidação terminal atômica (`QueueDriver::settle`, DATA-02).** O
  sucessor da chain e a confirmação agora commitam juntos no
  `DatabaseQueueDriver`, fechando a janela em que um crash entre os
  dois ou perdia o resto de uma chain, ou rodava seu próximo step duas
  vezes. A exclusão chaveada pela reserva serve também como fence: um
  worker cuja visibilidade expirou no meio da execução não commita
  nada e reporta `Settled::Stale`, então não consegue enfileirar
  trabalho para uma mensagem que outro consumer agora possui. Drivers
  que não conseguem fazer isso respondem `Settled::Unsupported` e
  mantêm a ordem documentada de push-antes-do-ack.
- **`DatabaseBatchRepository` (DATA-02).** A contabilidade de batch
  sobrevive a um restart, e `pending_jobs`/`failed_jobs` são derivados
  de linhas de liquidação chaveadas por `(batch_id, job_id)`, em vez de
  armazenados e decrementados - então um job redelivered não consegue
  levar um batch a "finished" enquanto seus outros jobs ainda estão
  rodando, e a salvaguarda vale entre processos, não só dentro de um.
- **`/_suprnova/health/live` e `/_suprnova/health/ready`.** Liveness
  não toca em nada; readiness sonda dependências. Conectar uma
  checagem de banco de dados numa sonda de liveness transforma um
  soluço de banco de dados num restart em cascata de toda réplica, o
  que o único endpoint anterior convidava.
  `/_suprnova/health` continua funcionando exatamente como
  documentado.
- **`SERVER_HEALTH_READINESS_TOKEN`.** Secret compartilhado opcional
  para a sonda de readiness, comparado em tempo constante. Sem ele,
  readiness responde 404 - indistinguível de um path não roteado,
  porque *é* o próprio 404 do router. Não definido por padrão, então
  sondas existentes continuam funcionando.
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`, com `ssl` e
  `null` aceitos como aliases compatíveis com o Laravel. Sem definir,
  deriva das credenciais, reproduzindo exatamente o comportamento
  anterior. Isso também torna alcançável o TLS implícito na porta 465:
  o transporte já suportava, mas nenhuma combinação de variáveis de
  ambiente conseguia selecioná-lo.
- **`SERVER_MAX_CONNECTIONS` e `SERVER_HEADER_READ_TIMEOUT`**
  documentados em `manual/env-vars.md`, onde estavam totalmente
  ausentes.

### Alterado

A própria conclusão da auditoria foi que o gate passou em 470s e não
pegou nenhum dos 19 P1s. A maior parte do trabalho de testes deste
release mira nisso.

- **Postgres roda no gate.** Doze testes em seis arquivos nunca
  tinham executado. Dois deles se revelaram apontando `DROP TABLE`
  para qualquer Postgres que estivesse em `localhost:5432` por
  padrão, e nenhum dos dois jamais tinha inicializado `Crypt`, então
  os dois falhavam na primeira vez que rodavam.
- **Asserções de scaffold leem os bytes que um usuário recebe**, após
  a substituição, em vez da fonte do template. Encontrou um projeto
  API entregando um doc comment nomeando um banco de dados
  literalmente `{package_name}`, e um `.env.example` anunciando cinco
  chaves de mail que o framework nunca lê.
- **Injeção de falha na fila.** Perda de ACK, redelivery, expiração de
  lease e dispatch parcial são dirigidos por um decorator que faz uma
  operação nomeada falhar numa chamada nomeada, então todo caso é
  determinístico em vez de uma corrida de sleep.
- **Adaptadores de pagamento têm testes negativos.** O `verify()` da
  Stripe nunca tinha sido exercitado com uma assinatura *válida*,
  então todo caminho de rejeição que depende de chegar à comparação
  HMAC não estava provado.
- **O driver do Pinecone fala REST.** *Incompatível, atrás da feature
  `vector-pinecone`, desligada por padrão.* A motivação está em
  **Segurança**; as mudanças de superfície são:
  - `client()` sumiu - não existe mais `PineconeClient`. No lugar
    ficam `control_plane_get`, `control_plane_post` e
    `data_plane_post`, que alcançam *qualquer* endpoint do Pinecone
    com seus próprios tipos de request e response sobre o transporte
    autenticado e com host resolvido do driver. Isso é estritamente
    mais alcance do que o trapdoor antigo tinha.
  - `json_to_metadata` → `metadata_from_json`, e metadata agora é
    `serde_json::Map` em vez de `prost_types::Struct`.
    `decode_match_fields` → `decode_match`, recebendo um
    `PineconeMatch`. `namespace()` retorna `&str`.
  - Novo: `with_control_plane`, `with_api_version`, `with_index_host`
    (fixa um host conhecido e pula o round trip do control plane),
    `index_host`, e os tipos de wire `PineconeVector` /
    `PineconeMatch`.
  - `from_env` ainda lê `PINECONE_API_KEY` e
    `PINECONE_CONTROLLER_HOST`, e agora também `PINECONE_API_VERSION`.
  - A versão da API REST é fixada, não flutuante - `2025-04`, a versão
    contra a qual as formas de request e response do driver foram
    escritas.
  - Nada mais serializa. O driver antigo cacheava um
    `Index` por nome atrás de um `tokio::Mutex` porque `pinecone-sdk`
    só o expunha atrás de `&mut self`; o novo cacheia uma string de
    host e compartilha o pool de conexões do `reqwest`.
  - Um host aprendido do control plane é sempre contatado sobre
    `https`, qualquer que seja o scheme que a resposta carregue.
  - `Debug` é implementado à mão com a API key redigida, então um
    `#[derive(Debug)]` numa struct que guarda um driver não consegue
    imprimi-la.
- **Testes de contrato de wire para o Pinecone.** Os testes de
  integração ao vivo precisam de uma `PINECONE_API_KEY` e por isso não
  conseguem rodar no gate - o que deixava os nomes de campo de uma
  reescrita REST (`topK`, `includeMetadata`, `vectorCount`) apoiados
  em nada. Treze testes agora dirigem o driver contra um fake
  `wiremock` local e verificam o método, path, headers e corpo JSON
  exatos que ele coloca na rede, mais que um não-2xx nunca é
  decodificado como resultado, e que uma mensagem de erro nunca
  carrega a API key. Eles fixam o driver ao contrato *documentado* do
  Pinecone; só os testes marcados `#[ignore]` conseguem confirmar que
  a documentação bate com o serviço ao vivo.

## 0.7.2 - 2026-07-28

### Corrigido

- **`generate-types` resolve structs de prop aninhados sem derives.** O
  gerador da 0.7.1 degradava para `unknown` todo campo de prop cujo
  tipo não derivasse `InertiaProps`/`Data` - então rodar de novo o
  gerador (ou o monitor do `suprnova serve`) sobre um projeto com um
  arquivo de types commitado substituía interfaces reais como
  `Array<AdminArticleRow>` por `unknown` e quebrava a checagem de
  tipos em todo o app. Structs simples definidos em qualquer lugar em
  `src/` agora resolvem para suas interfaces reais, transitivamente a
  partir das raízes de prop; `unknown` (com um aviso) fica reservado
  para tipos que o projeto genuinamente não define - tipos de crates
  externos, enums, tuple structs.

### Alterado

- **A geração de `routes.ts` é opt-in.** `generate-types` não deposita
  mais `frontend/src/types/routes.ts` em todo projeto sem ser pedido;
  passe `--routes` para gerá-lo.

- **Dependências dos starters de frontend atualizadas.** Scaffolds
  novos de `suprnova new` agora fixam versões atuais: Vite ^8.1.5,
  Tailwind CSS ^4.3.3, Svelte ^5.56.8 (vite-plugin-svelte ^7.2.0,
  svelte-check ^4.7.4), React ^19.2.8 (plugin-react ^6.0.4), Vue
  ^3.5.40 (plugin-vue ^6.0.8, vue-tsc ^3.3.8), e `@types/node` ^24 (a
  linha de types do Node 24 LTS). O TypeScript fica em ^6.0.3
  deliberadamente: é o mais recente da linha 6.x, e o range de peer do
  svelte-check (`^5 || ^6`) ainda não admite TypeScript 7. Os três
  starters foram verificados de ponta a ponta (`npm install` +
  `npm run build`) contra o conjunto atualizado.

## 0.7.1 - 2026-07-27

Uma passada de correção de defeitos sobre o roteamento de fila da
0.7.0, a partir de uma revisão completa pós-release.

### Corrigido

- **Jobs encadeados não perdem mais sua fila declarada.** `ChainLink`
  capturava o `max_tries`, `timeout` e `backoff` de um job no momento
  de construção da chain, mas não seu `Job::queue()`, então um job que
  caía na fila declarada quando empurrado diretamente caía em
  `default` quando despachado como parte de uma chain - o nível "job"
  da ordem de resolução rota → job → default sumia silenciosamente
  para chains. A fila declarada agora é capturada no link e resolvida
  exatamente como um push direto. Payloads de chain escritos antes
  deste release decodificam sem alteração (`serde(default)`), e um
  link sem fila declarada serializa byte-idêntico ao que a 0.7.0
  escrevia.
- **Registros de failed-job carregam a fila em que o job morreu.** O
  caminho de dead-letter do worker fixava `queue = "default"` em todo
  registro `FailedJob`, então falhas de um job roteado eram invisíveis
  para um operador filtrando o store de falhas pelo pool que as
  possui. O registro agora carrega a fila do envelope (`default` para
  jobs não roteados).
- **A nota de upgrade da 0.7.0 subestimava a migração de `jobs`.**
  Dizia "workers sem filtro não são afetados e não precisam de
  migração", mas `DatabaseQueueDriver::push` nomeia a coluna `queue`
  em seu `INSERT` esteja o job roteado ou não - um binário 0.7.0
  contra uma tabela não migrada falha **todo push**, filtrado ou não.
  A seção 0.7.0 abaixo e `manual/queues.md` estão corrigidas: no
  driver de banco de dados o `ALTER TABLE` é obrigatório para todo
  deployment, e precisa rodar antes de os binários subirem (binários
  mais antigos listam suas colunas explicitamente, então migrar
  primeiro é seguro).

- **O README não anuncia mais uma macro `#[job]`.** Nenhuma macro
  dessas existe - jobs implementam a trait `Job`. A linha de filas
  agora descreve a superfície real, incluindo o roteamento de fila da
  0.7.0.

### Alterado

- **O caminho de release agora atualiza as referências de versão do
  README.** `bump-workspace-version.py` reescreve a tag de instalação
  fixada do README, o exemplo de modelo de distribuição, e a linha de
  MSRV atomicamente com os manifestos, e um README reformulado que
  para de casar com um padrão falha o release de forma explícita. O README
  vinha anunciando a v0.6.0 desde que a v0.7.0 foi lançada, porque nada
  no caminho de release o tocava.
- **O roteamento de conexão é documentado como só resolução de nome.**
  `Job::connection()` e o campo de conexão de `Queue::route` resolvem
  o *nome* de conexão carregado nos eventos de ciclo de vida
  `JobQueueing` / `JobQueued`; um único driver global de processo
  ainda recebe todo push, então eles não selecionam um driver
  diferente. O rustdoc e `manual/queues.md` antes davam a entender uma
  seleção de driver que não existe. A dimensão de fila não é afetada -
  ela é honrada de ponta a ponta. Drivers por conexão continuam sendo
  trabalho futuro.
- `ChainLink` ganhou um campo público `queue: Option<String>`, o que
  quebra a construção por struct literal de links de chain. Links
  construídos através de `ChainLink::from_job` - o caminho normal -
  não são afetados.

### Atualizando

Vindo de ≤ 0.6.x no driver de fila de banco de dados, aplique a
migração da 0.7.0 abaixo **antes** de subir os binários; ela é
obrigatória para todo deployment nesse driver, não só os que usam
`--queue`. A própria 0.7.1 não precisa de migração nenhuma.

## 0.7.0 - 2026-07-26

### Segurança

- **`ammonia` atualizado para 4.1.4 (RUSTSEC-2026-0213).** Versões até
  a 4.1.3 permitem XSS via tags de animação SVG `animate` e `set`.
  `ammonia` é o sanitizer no fim do pipeline de markdown do Suprnova
  (`comrak` → `syntect` → `ammonia`), então todo app renderizando
  Markdown fornecido por usuário através de `content` estava exposto.
  O aviso foi publicado em 2026-07-21 - depois que a v0.6.5 foi
  lançada - então **todo release até e incluindo a v0.6.5 é afetado**.
  Atualizar o framework é a correção; nenhuma mudança de código de
  aplicação é necessária.

### Adicionado

- **Roteamento de fila.** Jobs podem ser despachados para uma fila e
  conexão específicas, e workers podem ser dedicados a filas
  específicas - a superfície `Queue::route(...)` do Laravel 13,
  tipada. Um job declara sua própria casa com `Job::queue()` /
  `Job::connection()`; um operador a sobrescreve centralmente com
  `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` em
  `bootstrap::register()`, sem editar o job. A resolução é rota,
  depois job, depois default global, e um campo `None` numa rota
  adia em vez de limpar. `queue:work --queue=billing,default` drena
  só aquelas filas. Jobs não roteados pertencem a `default`, então
  nunca ficam encalhados. Jobs encadeados resolvem rotas por nome, já
  que um link de chain guarda seu job apagado (erased).
- **`QueueDriver::pop_from`.** Pop com filtro, com uma implementação
  padrão que **rejeita** um filtro que não consegue honrar, em vez de
  silenciosamente drenar toda fila - um worker instruído a drenar
  `billing` que silenciosamente drena tudo é indistinguível de um
  deployment funcionando até o pool errado comer os jobs errados. Os
  drivers de memory e database filtram nativamente. Drivers
  personalizados continuam compilando e herdam o padrão explícito.
- **Schema da tabela `jobs` documentado.** `manual/queues.md` agora
  carrega o DDL que `DatabaseQueueDriver` de fato espera, o que antes
  só era descobrível lendo o SQL do driver.
- **Opção `serverHead` do Inertia documentada.** Elementos `<head>`
  dirigidos pelo servidor (Inertia 3.5.0) não precisam de suporte
  nenhum do framework: o cliente os lê de uma prop comum, então
  qualquer handler já pode fornecê-los. Veja
  `manual/frontend-inertia-responses.md`.

### Alterado

- `Envelope` ganhou um campo `queue: Option<String>`. É
  `serde(default)` e pulado quando ausente, então um envelope não
  roteado serializa byte-idêntico ao que versões anteriores
  escreviam - o teste de wire-format congelado passa sem alteração,
  não há bump de `schema_version`, e frotas de versão mista
  interoperam durante um upgrade rolling.
- `WorkerConfig` ganhou um campo `queues: Vec<String>` (vazio = drena
  tudo, o comportamento anterior).
- Removido `ROADMAP.md`. Seus princípios de design vivem em
  `manual/introduction.md`, o acordo de trabalho em
  `manual/contributions.md`, e o material de deployment e scale-out
  em `manual/deployment.md`; as checklists de shipped/planned tinham
  ficado desatualizadas. O ponteiro do `README.md` para ele, para "a
  relação com o upstream", já estava quebrado - essa atribuição vive
  em `LICENSE`.
- Frontends de scaffold agora fixam `@inertiajs/{svelte,react,vue3}`
  em `^3.6.1` (a partir de `^3.4.0`). O intervalo 3.4.0 → 3.6.1 é só
  client-side - auditado contra o changelog upstream e o contrato
  `Page` em `packages/core/src/types.ts`, todo header `X-Inertia-*`
  que o cliente 3.6.1 envia já era tratado.
- `scripts/release.sh` agora publica o próprio release do GitHub, com
  notas tiradas da seção `CHANGELOG.md` da versão. Antes isso era um
  "próximo passo" manual que ficava sendo pulado, motivo pelo qual a
  v0.5.10 e a v0.6.1-v0.6.3 são só-tag e a página de Releases ficou
  numa versão desatualizada. O preflight roda antes do gate, então um
  `gh` ausente ou uma seção de changelog ausente falha em segundos, e
  a publicação é pulada automaticamente a menos que `origin` seja o
  GitHub.

### Atualizando

Tabelas `jobs` existentes no driver de fila de banco de dados
**precisam** adicionar a coluna nova - `push` a nomeia em seu
`INSERT` esteja o job roteado ou não, então uma tabela não migrada
falha todo push. Migre primeiro, depois suba os binários (binários
mais antigos listam suas colunas explicitamente e ignoram a nova,
então essa ordem é segura):

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*(Corrigido na 0.7.1 - esta nota originalmente afirmava que
deployments sem filtro não precisavam de migração.)*

## 0.6.5 - 2026-07-21

### Adicionado

- **Checkout avulso hospedado no adaptador Stripe.**
  `Checkout::start_session` com `SessionMode::OneOff` e `price_refs`
  não vazio agora cria uma Checkout Session hospedada
  (`mode=payment`, um line item por price ref,
  `allow_promotion_codes=true`) e retorna
  `SessionPayload::StripeCheckoutRedirect`. O caminho Elements só com
  `amount_hint` fica inalterado; as duas formas são escolhidas por
  solicitação.
- **Suporte a Stripe Managed Payments (merchant-of-record).**
  `StripeProvider::with_managed_payments(true)` - ou
  `STRIPE_MANAGED_PAYMENTS=true` em `from_env()` - envia
  `managed_payments[enabled]=true` na criação de sessão avulsa
  hospedada. Desligado por padrão; o campo é totalmente omitido, então
  contas não cadastradas não são afetadas.
- **`Checkout::session_status`.** Novo método de trait (padrão:
  `PaymentError::NotSupported`) reportando o estado do lado do
  provider de uma sessão como o novo `CheckoutSessionState` neutro
  (`Open` / `Complete { paid, payment_ref, amount_total }` /
  `Expired`). A impl da Stripe mapeia
  `GET /v1/checkout/sessions/{id}`; `payment_ref` carrega o id do
  PaymentIntent da sessão para correlação com a tabela espelho. Esta
  é a primitiva de verificação do lado do servidor para páginas de
  retorno de redirect e varreduras de reconciliação.
- **Trait de capacidade `Promotions`.** `create_promotion_code` cunha
  um código restrito a um cliente, opcionalmente expirável, com
  limite de resgates, a partir de um cupom pré-criado. Consultada via
  o novo `PaymentProvider::as_promotions()` (padrão `None`).
  Implementada para a Stripe (`POST /v1/promotion_codes`) e para o
  mock.
- **Atualizações do `MockPaymentProvider` para o acima.** Registra
  toda solicitação `start_session` (`recorded_sessions()`), roteiriza
  `session_status` por id de sessão (`script_session_status()` -
  sessões conhecidas sem roteiro reportam `Open`, ids desconhecidos
  `NotFound`), e implementa `Promotions` com solicitações registradas
  (`recorded_promotion_requests()`).

## 0.6.4 - 2026-07-17

### Corrigido

- **Agregados Eloquent decodificam de forma consistente entre backends
  de banco de dados.** Expressões geradas de `count`, `sum`, `avg`,
  `min` e `max` agora usam um único alias interno estável de
  resultado. O PostgreSQL não retorna mais zeros falsos ou `None`
  porque seu driver rotula colunas agregadas de forma diferente do
  SQLite, e erros de coluna ausente ou tipo incompatível agora se
  propagam em vez de serem silenciosamente substituídos por um
  default.
- **Exclusões em massa não podem usar expressões de tabela fornecidas
  pelo caller.** O SQL de exclusão executável sempre deriva seu alvo
  do `M::TABLE` estático validado do model. O argumento legado
  público do renderer continua compatível na fonte, mas não consegue
  redirecionar ou injetar o alvo da exclusão.

## 0.6.3 - 2026-07-15

### Adicionado

- **Leituras raw tipadas podem ficar na conexão fixada de uma
  transação.** `Transaction::backend()` expõe o backend ativo e
  `Transaction::query_all(Statement)` executa SQL agregado tipado ou
  personalizado através da transação, preservando a instrumentação
  `QueryExecuted`. Aplicações não precisam mais de uma consulta no
  nível do pool ou de acesso a executor privado quando uma decisão com
  escopo de lock depende de colunas de resultado computadas.

## 0.6.2 - 2026-07-15

### Corrigido

- **Predicados raw vinculados são neutros quanto ao backend.** O
  `filter_raw` e o `where_raw` do Eloquent agora aceitam marcadores de
  bind `?` portáveis em todo backend de banco de dados; a
  renderização do PostgreSQL os rebaseia para posições `$N`
  monotônicas ao longo de predicados anteriores, subconsultas de
  relacionamento, cláusulas HAVING e ramos de UNION. Fragmentos
  numerados existentes do PostgreSQL são normalizados pela sua ordem
  local de marcadores, enquanto estilos misturados e incompatibilidades
  entre contagem de marcadores falham a validação antes do I/O. O
  scanner sensível a SQL preserva pontos de interrogação dentro de
  strings entre aspas, identificadores, comentários e corpos com
  dollar-quoting; `??` emite um operador literal de ponto de
  interrogação num fragmento raw vinculado.

## 0.6.1 - 2026-07-15

### Adicionado

- **Limpeza de sessão supervisionada e observável.**
  `SessionMiddleware::install` usa a cadência configurável de
  `SESSION_GC_INTERVAL` (uma hora por padrão), enquanto
  `session_gc_metrics()` expõe execução, sucesso, falha, linhas
  removidas e timestamps do último resultado, locais ao processo,
  para superfícies de operações protegidas.
- **Touches de sessão deslizante limitados.** `SESSION_TOUCH_INTERVAL`
  controla a cadência mínima de escrita de atividade (cinco minutos
  por padrão) e é limitado à metade do tempo de vida da sessão, para
  que sessões ativas não possam expirar entre touches.

### Corrigido

- **Solicitações sem estado não criam mais sessões duráveis.**
  Solicitações sem um cookie de sessão válido não fazem leitura nem
  escrita no session store, e não recebem cookie de sessão, a menos
  que o tratamento crie estado. Sessões limpas existentes evitam
  upserts incondicionais e churn de cookie, cookies legados migram na
  próxima solicitação, e cookies cujas linhas de apoio expiraram são
  limpos sem recriar sessões vazias.

## 0.6.0 - 2026-07-10

### Adicionado

- **Subsistemas do framework opt-in, com padrões compatíveis com
  versões anteriores.** O storage de filesystem, os drivers de banco
  de dados SQLite/Postgres/MySQL, o driver de vetor do MariaDB, e o
  Web Push agora têm features explícitas do Cargo. Builds padrão
  existentes mantêm todas essas capacidades, enquanto consumidores com
  `default-features = false` podem selecionar zero drivers ou só a
  superfície de storage/banco de dados/vetor/push que usam. A matriz
  de features executável verifica os perfis zero-driver,
  driver-individual, Nation X mínimo, padrão e all-feature.
- **Importação de chave privada VAPID P-256 crua.** `VapidKey::from_bytes`
  aceita um escalar P-256 big-endian de 32 bytes validado, ao lado do
  caminho existente de import/export PKCS#8 PEM.

### Alterado

- **JWTs VAPID são assinados diretamente com P-256.** O Web Push agora
  serializa o header/claims ES256 do RFC 8292 e os assina com `p256`,
  removendo a dependência genérica de JWT, preservando as chaves
  geradas, os round trips de PEM, a codificação de chave pública, e o
  limite de tempo de vida de 24 horas.
- **Atualização de dependências de segurança.** Dependências
  vulneráveis do framework atualizadas, incluindo bcrypt e ammonia, e
  as features ativadas do Comrak estreitadas, mantendo o realce de
  sintaxe.
- **Rust 1.91.1 é o MSRV do release.** Todo pacote do workspace
  declara o mesmo `rust-version`, Dockerfiles gerados fixam a imagem
  de builder correspondente, e o gate de release completo compila o
  perfil de filesystem suportado com o toolchain exato do Rust
  1.91.1.
- **Fixação de segurança do OpenDAL 0.58.** A feature de filesystem
  fixa o commit `88717391eb72c9839d3f8e79fccad9f22fc3a1b4` de
  `eas4ai/opendal`, um fork mínimo baseado exatamente no
  commit oficial `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` do Apache
  OpenDAL. O fork muda só as declarações do Reqsign usadas pelo core
  do OpenDAL mais S3, GCS e Azure Blob, para que consumidores
  downstream resolvam o commit oficial `b49cd2996b9d2d9944e84481f8835ff55b188b97`
  do Apache Reqsign e `quick-xml` 0.41.0. Um fork é necessário porque
  os patches de Cargo na raiz de um repositório de dependência não se
  propagam para os consumidores; o grafo publicado, do contrário,
  poderia restaurar o `quick-xml` 0.38/0.40 vulnerável.

### Corrigido

- **Metadados de versão de release atômicos.** O bump de release agora
  atualiza `workspace.package.version` e toda dependência de path
  interna versionada numa única operação validada, coloca no stage
  todo manifesto afetado, e prova um workspace `0.6.0` temporário com
  `cargo check --workspace` antes do release. Versões de release são
  validadas como SemVer 2.0 estrito, incluindo a regra de zero à
  esquerda para prerelease numérico. Smokes descartáveis
  agnósticos-a-versão em remote nu derivam um patch release posterior
  tanto da fonte atual quanto de uma fonte já em `0.6.0`, rejeitam
  árvores de release staged/unstaged/untracked antes do gate, provam
  que a publicação atômica de commit/tag reverte as duas referências
  quando uma tag é rejeitada, e provam a sequência normal de release
  sem tocar no remote real. Versões de release precisam aumentar por
  precedência SemVer, incluindo transições de prerelease. Artefatos de
  build do smoke sempre ficam dentro do seu workspace temporário,
  ignorando qualquer `CARGO_TARGET_DIR` do caller.
- **O rustdoc cobre toda fronteira de feature suportada.** O módulo
  OAuth linka para o `OAuthAuth::complete` público, e a matriz
  executável constrói rustdoc zero-driver, padrão e all-feature sem
  dependências.
- **A validação de stream de filesystem tem escopo de sessão.**
  Writers, listers e copiers de filesystem local resolvem e confinam
  seus paths uma vez antes do primeiro I/O, em vez de uma vez por
  chunk/item, enquanto operações ativadas de close/abort sempre
  alcançam o backend para limpeza. O confinamento existente de
  traversal e symlink continua aplicado para um filesystem confiável;
  checagens de canonicalize-então-open não eliminam corridas contra um
  principal mutando a árvore concorrentemente.

### Segurança

- **O gate de release falha de forma fechada.** `release.sh` delega
  para o gate completo canônico antes de editar manifestos ou criar
  commits/tags; esse gate sempre roda `cargo audit`, trata um binário
  `cargo-audit` ausente como um erro, e para em qualquer falha de
  auditoria. Também constrói e audita um consumidor de filesystem
  downstream isolado, garantindo revisões exatas de fonte do
  OpenDAL/Reqsign e nenhum `quick-xml` abaixo de 0.41. Nenhuma
  exceção de aviso nova foi adicionada.

## 0.5.10 - 2026-07-03

### Corrigido

- **`generate-types` não descarta mais structs autorreferentes.** Um
  struct com um campo que referencia seu próprio tipo (um nó de árvore
  com `children: Vec<Self>`, por exemplo uma view de comentários em
  thread) criava uma self-edge no grafo de dependência de tipos,
  fixando seu in-degree acima de zero, então a ordenação topológica de
  Kahn nunca o emitia - deixando toda interface que o referenciasse
  com um nome de tipo pendurado que falhava no `svelte-check`/`tsc`.
  Self-edges agora são removidas antes da ordenação, e quaisquer
  structs presos num ciclo de referência (recursão mútua) são
  emitidos em ordem arbitrária em vez de descartados, já que
  interfaces TS podem referenciar umas às outras independentemente da
  ordem de declaração.

## 0.5.9 - 2026-07-01

### Adicionado

- **`MAIL_FROM_NAME` - nome de exibição opcional nos e-mails de
  auth-flow.** Os mailables de verificação de e-mail, redefinição de
  senha e senha alterada agora renderizam seu header `From` como
  `"Name <address>"` quando `MAIL_FROM_NAME` está definido (lido no
  momento do envio, então sobrevive ao round-trip de serde da fila).
  `MAIL_FROM` continua sendo um endereço puro; deixar `MAIL_FROM_NAME`
  sem definir ou em branco mantém o comportamento anterior de
  endereço puro. Nenhuma mudança em nenhum call site - os próprios
  mailables leem a env var.

## 0.5.8 - 2026-06-30

### Corrigido

- **Os route helpers do `generate-types` agora são sempre TypeScript
  válido.** Quando várias rotas num módulo compartilham um handler
  (por exemplo uma whitelist de `static_files::serve` mapeando várias
  URLs de favicon/asset), a primeira mantinha o nome do handler e as
  demais recebiam uma chave derivada do path da rota - mas o path era
  só parcialmente sanitizado (`/ { } -` → `_`), então uma extensão de
  arquivo vazava um `.` para dentro da chave: `favicon_16x16.png:
  (...) => ...`. Isso é acesso a membro, não um nome de propriedade,
  então `tsc`/`svelte-check` rejeitavam o `routes.ts` gerado. Chaves
  derivadas agora são sanitizadas para identificadores válidos - todo
  caractere não alfanumérico vira `_` e um dígito inicial recebe um
  prefixo - então `favicon-16x16.png` → `favicon_16x16_png` e
  `2fa.json` → `_2fa_json`. Nomes de handler únicos ficam intocados.

## 0.5.7 - 2026-06-30

### Corrigido

- **`generate-types` não emite mais referências de tipo penduradas.**
  Um campo de prop cujo tipo é um struct que não deriva
  `InertiaProps`/`Data` (ou um tipo externo que o gerador não
  consegue ver) era emitido como um identificador solto - por exemplo
  `user: UserInfo` - produzindo TypeScript que falha no
  `tsc`/`svelte-check` porque essa interface nunca é escrita. Tais
  referências agora degradam para `unknown` (`user: unknown`;
  `Vec<T>` → `Array<unknown>`; `Option<T>` → `unknown | null`), então
  a saída gerada sempre passa na checagem de tipos, e `generate-types`
  imprime um aviso nomeando o tipo não resolvido e o campo que o
  referencia, com a correção (derivar `InertiaProps`/`Data` nele).
  Parâmetros genéricos e tipos InertiaProps/Data aninhados resolvidos
  não são afetados.

## 0.5.6 - 2026-06-29

### Alterado

- **Sign in with Apple: verificação JWKS RS256.** Bump do
  `suprnova-apple-rs` para v0.3.1 - tokens de ID da Apple agora são
  verificados contra o JWKS publicado da Apple (RS256) em vez de
  confiados estruturalmente.

## 0.5.5 - 2026-06-28

### Adicionado

- **Propósito de token `MagicLink`.** Nova variante `MagicLink` no
  enum `TokenPurpose` de auth-flow, para tokens de login sem senha por
  magic link.

## 0.5.4 - 2026-06-28

### Alterado

- **Conclusão de OAuth componível.** Divide a conclusão genérica de
  OAuth em `verify_oauth_identity` (verifica + resolve a identidade) e
  um `complete` fino, para que apps consigam verificar uma identidade
  OAuth sem disparar todos os efeitos colaterais da conclusão de
  sessão.

## 0.5.3 - 2026-06-28

### Corrigido

- **Metadados de versão de workspace corrigidos.** A v0.5.2 foi
  taggeada e enviada antes de o bump de versão do seu `Cargo.toml` ser
  colocado no stage, então a tag v0.5.2 enviada ainda lê `version =
  "0.5.1"`. A v0.5.3 recorta o release com a versão de workspace
  correta - nenhuma mudança de código (a divisão de OAuth da v0.5.2
  não é afetada).

## 0.5.2 - 2026-06-28

### Alterado

- **Conclusão de Apple componível.** Divide a conclusão do Sign-In da
  Apple em `verify_apple_identity` + um `complete_apple` fino,
  espelhando a divisão genérica de OAuth. (Nota: a tag v0.5.2 enviada
  carrega um campo de versão `0.5.1` desatualizado - corrigido na
  v0.5.3.)

## 0.5.1 - 2026-06-28

### Alterado

- **Crate da Apple renomeado.** Reaponta a dependência da Apple para o
  repositório renomeado `suprnova-apple-rs`.

## 0.5.0 - 2026-06-28

### Adicionado

- **Sign in with Apple.** Troca de token OAuth + verificação de ID
  token + upsert de usuário para a Apple; endpoints well-known da
  Apple e o modo de resposta `form_post`; campos específicos da Apple
  em `OAuthProviderConfig`; `AppleKeyPair` reexportado para que apps
  configurem o Sign-In with Apple sem uma dependência direta de
  `apple`.

### Corrigido

- Omite parâmetros PKCE da URL de authorize da Apple (a Apple rejeita
  a solicitação quando eles estão presentes).

### Dependências

- Consome a correção de magic-auth do `torii`; adiciona `apple-rs`
  v0.3.0.

## 0.4.1 - 2026-06-26

### Desempenho

- Pré-dimensiona `MiddlewareChain` para eliminar realocações de `Vec`
  por solicitação.

### Corrigido

- Torna o path do arquivo de manutenção (down-file) à prova de colisão
  sob execuções de teste paralelas.

### Docs

- Compile-checa os exemplos de doc do framework (`ignore` → `no_run`);
  reconcilia as notas de distribuição com as GitHub Releases taggeadas;
  ignora a árvore `docs/` inteira.

## 0.4.0 - 2026-06-22

### Alterado

- **A distribuição é rastreada por git; você não fixa em tags.** Apps
  com scaffold dependem de `suprnova = { git = "…/suprnova.git" }` e
  seguem a branch padrão; puxe atualizações com `cargo update -p
  suprnova`. Versões são publicadas como GitHub Releases taggeadas
  (`v0.4.0`, …) para o changelog, mas `Cargo.lock` já fixa o commit
  exato resolvido - então builds continuam reprodutíveis sem fixar
  `tag` ou `rev` à mão. A documentação de instalação não apresenta
  mais a fixação por commit como o caminho de atualização.

## 0.3.0 - 2026-06-21

### Adicionado

- **Instrumentação de query para leituras Eloquent** - `Builder::get`,
  `Model::find`, `find_many`, e `all` agora emitem `QueryExecuted`,
  então SELECTs de model e queries de eager-load aparecem em
  `DB::listen` e no log de query em memória junto com escritas e
  queries raw. Adiciona o terminal de leitura instrumentado
  `ExecutorChoice::statement_all`.
- **Autorização de rota de recurso** -
  `ResourceRoutes::authorize_resource::<U, R>()` anexa a checagem de
  habilidade convencional a toda rota de recurso gerada, como
  middleware por rota (paridade com o `authorizeResource` do
  Laravel). O mapa ação→habilidade é `index`/`show` → `view`,
  `create`/`store` → `create`, `edit`/`update` → `update`,
  `destroy` → `delete`. Uma única chamada faz gate na superfície
  inteira de sete ações, em vez de depender que todo corpo de
  controlador lembre de um `Gate::authorize`.
- **Hit atômico de rate limit** - `RateLimiter::hit_and_check(key, max,
  decay)` incrementa uma janela fixa e a testa num único round-trip,
  retornando se o bucket agora está acima do seu limite (`i64::MAX`
  significa ilimitado).
- **Helper de comparação em tempo constante** - `constant_time_eq(a,
  b)` (apoiado em subtle) para verificação de assinatura de webhook;
  a documentação de `WebhookHandler::verify` agora exige comparação de
  digest em tempo constante.
- **Cliente Inertia para 3.4.0** - os scaffolds Svelte/React/Vue agora
  fixam `@inertiajs/{svelte,react,vue3}` em `^3.4.0` (a partir de
  `3.1.1`), ganhando os modos `router.poll`, `usePoll` dinâmico,
  `Inertia.once`, a correção de cancelamento do InfiniteScroll, e o
  `onSuccess` aguardado do Form. O servidor já emite a superfície
  completa de objeto de página e headers da 3.4.0 (once-props, a
  família de scroll prepend/deep-merge, `matchPropsOn`, props
  resgatadas/compartilhadas), então isso é um bump de atualidade do
  cliente sem mudança de protocolo.
- **Limite de conexão opcional** - `SERVER_MAX_CONNECTIONS` (e o
  `Server::max_connections(n)` programático) limita conexões
  concorrentemente ativas com um semáforo no accept loop, aplicando
  contrapressão no nível TCP. Sem definir - ou `0` - deixa as conexões
  sem limite (o padrão, inalterado). Um backstop para parear com um
  proxy reverso e `LimitNOFILE`, não um substituto para rate limiting
  upstream.
- **Opção de não seguir redirects** - `RequestBuilder::no_redirects()`
  roteia uma solicitação através de um cliente HTTP que não segue
  redirects, então um `3xx` é retornado como está, em vez de
  perseguido. Use quando a URL da solicitação é influenciada por
  entrada não confiável, para fechar um vetor de SSRF baseado em
  redirect (um endpoint hostil redirecionando para um host interno ou
  de metadados de nuvem). O cliente padrão continua seguindo
  redirects, seguindo a convenção geral de cliente.

### Segurança

- **Rotas de recurso** falham de forma fechada no downcast type-erased
  do registro de autorização em vez de entrar em panic, e negações de
  `authorize_resource` / solicitações não autenticadas são recusadas
  antes do handler rodar.
- **O rate limiter** fecha uma corrida de check-then-hit de janela
  fixa incrementando e comparando atomicamente (`hit_and_check`).
- **O middleware `RateLimited` de fila** agora admite jobs através
  daquele `hit_and_check` atômico, em vez de um par separado de
  `too_many_attempts` + `hit`, então workers concorrentes não
  conseguem mais todos passar na checagem de orçamento antes de
  qualquer um deles incrementar, e super-admitir além de
  `max_attempts`.
- **Validadores de upload** (`mimetypes` / `mime`) fazem content-sniff
  dos bytes enviados em vez de confiar no `Content-Type` fornecido
  pelo cliente.
- **A guarda de path de filesystem** canonicaliza paths para pegar
  traversal por symlink para fora da raiz de storage, além das
  checagens léxicas anteriores de `../` / absoluto / UNC.
- **Auth** fecha um oráculo de timing de login sem senha - uma conta
  casada mas sem senha, recebendo uma senha, agora roda uma
  verificação de custo fixo, tanto no provedor de usuário Eloquent
  quanto no de banco de dados - e `dummy_verify` dirige o hasher
  configurado, então o caminho de usuário não casado é de tempo
  constante.
- **Eloquent** valida identificadores de coluna nos caminhos de
  projeção de `pluck` / `value` / `pluck_keyed` / `sole_value` e
  `sum` / `avg` / `min` / `max`.
- **Pagamentos** - o verificador do provider mock falha de forma
  fechada fora de um ambiente de desenvolvimento, e IPs de origem de
  webhook resolvem através de `TrustedProxiesConfig` (`req.ip()`), em
  vez de um header `X-Forwarded-For` bruto.
- **A guarda de path de filesystem** agora caminha até o ancestral
  *existente* mais próximo quando um alvo de escrita ainda não
  existe, fechando um escape por symlink em que um symlink
  intermediário plantado com um pai imediato ausente escapava da
  guarda.
- **`DB::init_with`** valida o ambiente antes de conectar (casando com
  `DB::init`), então o fallback de SQLite de dev não consegue mais
  inicializar silenciosamente em produção por essa porta de entrada.
- **A entrega de arquivo estático** rejeita dotfiles (`.env`,
  `.git/config`, `.htpasswd`, qualquer segmento começando com `.`),
  não só traversal de `.`/`..`.
- **Webhooks de pagamento** serializam retries concorrentes do mesmo
  evento não processado com um lock `FOR UPDATE` + reverificação, e
  tratam violações de unique na tabela espelho como já-aplicado
  benigno; `payments_subscription_items` ganha um
  `UNIQUE(subscription_id, provider_item_id)`.
- **RBAC** usa por padrão o nome de tipo totalmente qualificado como
  discriminador de model, então dois tipos autenticáveis
  compartilhando um nome de folha não conseguem mais herdar os
  papéis/permissões um do outro.
- **`invalidate_session()`** rotaciona o id de sessão (não só faz
  flush), fechando uma brecha de fixação de sessão; o middleware
  `WithoutOverlapping` de fila libera seu lock de cache mesmo quando o
  job entra em panic.
- **Providers de mail** limitam a leitura do corpo de resposta de erro
  (8 KiB), casando com o cliente de web push, então um endpoint
  hostil não consegue drenar a memória do remetente.
- **O web push** desativa o seguimento de redirect HTTP no cliente
  padrão, então um endpoint de push influenciado por atacante não
  consegue mais redirecionar `3xx` um POST de notificação para um
  host interno ou de metadados de nuvem (SSRF). Um redirect agora
  aparece como um push rejeitado, em vez de uma solicitação seguida
  silenciosamente.
- **O adaptador Stripe** redige o secret de assinatura de webhook no
  `Debug` *e* imprime um placeholder para o `stripe::Client` (que
  carrega a API secret key no seu header de auth), então nenhum
  secret consegue chegar aos logs através de um `{:?}` de
  `StripeProvider`, independentemente do próprio `Debug` do cliente
  upstream.
- **O adaptador Stripe** `from_env` rejeita credenciais
  presentes-mas-em-branco, falhando de forma fechada em vez de
  construir um cliente com um secret HMAC de webhook vazio (e,
  portanto, forjável).
- **A verificação de e-mail OAuth** falha de forma fechada para
  providers não reconhecidos: um payload de userinfo carregando um
  `email`, mas sem flag `email_verified`, não é mais tratado como
  verificado. Um provider desconhecido agora precisa afirmar
  `email_verified: true` ou expor um endpoint de e-mails verificados,
  fechando um vetor de vínculo/takeover de conta para apps que
  chaveiam contas por e-mail. Google (só-`true`-explícito) e GitHub
  (verificado pelo contrato do `/user`) não são afetados.

### Corrigido

- **O eager loading aninhado** (`with(["posts.comments"])`) agora é um
  número constante de queries - o segmento final carrega numa única
  query IN em lote ao longo de todos os pais, em vez de uma query por
  pai (N+1).
- **`where_has`/`where_doesnt_have`** qualificam colunas de closure com
  a tabela alvo, então uma coluna presente tanto no pivot quanto no
  alvo não produz mais um erro de coluna ambígua em relações
  many-to-many.
- **O `delete`/`force_delete`/`touch` de soft-delete e o `persist` de
  factory** honram o roteamento `#[model(connection = "…")]` de um
  model (casando com `restore` e os outros caminhos de escrita) em
  vez de cair de volta no pool primário.
- **O `Maybe::Missing` do JSON:API** usa uma sentinela de wire
  não-colidível, então dados de usuário no formato
  `{"__missing__": true}` não são mais silenciosamente removidos.
- **Notificações enfileiradas** honram `should_send` (veto por canal)
  e `after_sending`, reverificados no worker - antes só o caminho
  síncrono fazia isso.
- **Jobs released** empurram a cópia de retry antes de dar ack no
  original, então um erro transiente de push do driver não descarta
  mais o job.
- **Webhooks de ajuste (reembolso) da Paddle** chaveiam a atualização
  espelho pelo id de transação referenciado e leem os valores de
  `data.totals`, em vez de inserir uma linha de valor zero sob o id do
  ajuste.
- **URLs SQLite** carregando uma query string
  (`sqlite://db.sqlite?mode=rwc`) constroem uma URL de conexão de
  query única válida e um nome de arquivo em disco limpo.
- **HTTP** limita valores `q` de `Accept` a `[0,1]` e aplica o
  `max_body_bytes` de um `FormRequest` mesmo quando o corpo foi
  pré-bufferizado; a config de **WebSocket** rejeita
  `max_missed_pings < 2` (1 fechava toda conexão no seu primeiro
  ping).
- **Cron** usa semântica OR para dia-do-mês e dia-da-semana quando os
  dois são restritos (paridade Vixie/POSIX); `plain_text`/excertos de
  Markdown preservam pontuação espaçada intencional; `CachedEvaluator`
  limita o crescimento do seu cache; `SupervisorRegistry::start_all`
  não faz mais double-spawn numa segunda chamada; o contêiner de teste
  se recupera no lugar de um lock envenenado.
- **O backoff de restart do supervisor** volta ao piso de 100 ms
  depois de uma execução que fica de pé por pelo menos o teto de 60
  s, então um daemon que rodou saudável por um longo período e depois
  sai reinicia prontamente, em vez de herdar um backoff que subiu
  durante uma rajada de falhas anterior. Um crash loop cujas execuções
  nunca alcançam o limiar continua subindo até o teto, então o reset
  nunca mascara um supervisor instável.
- Corrigida documentação desatualizada sobre `filter_op` (operadores
  são validados por allowlist), URLs assinadas (não compatíveis byte a
  byte com as assinaturas absolutas padrão do Laravel),
  `UniqueIdKind::is_valid` (um helper de caller, não conectado
  automaticamente em `find`), e o limite de tamanho de identificador
  (128, não 64).

### Documentação

- Documentada a autorização de rota de recurso (`authorize_resource`)
  nos capítulos de roteamento e autorização, e o contador atômico
  `hit_and_check` no capítulo de rate limiting.

## 0.2.0 - 2026-06-21

Adiciona controle de acesso baseado em papéis, um pipeline de
renderização de conteúdo Markdown / docs, e entrega nativa de
arquivo estático.

### Adicionado

- **RBAC Tier-2** - trait `HasRoles`; papéis + permissões com um join
  `role_has_permissions`; `PermissionMiddleware` / `RoleMiddleware`
  (os dois fail-closed / default-deny); a migração
  `CreateRbacTables`; e os helpers `create_role` /
  `create_permission` / `give_permission_to_role`.
- **Renderização de conteúdo** - renderização de Markdown e um
  pipeline de build de docs: `MarkdownRenderer`, `build_docs`,
  `DocsCatalog` / `DocsChapter`, extração de heading e
  `slugify_heading`. O HTML renderizado é sanitizado
  (comrak + syntect + ammonia).
- **Entrega nativa de arquivo estático** - handler de fallback
  `StaticFiles::public()` para servir um diretório `public/` na raiz
  web, substituindo controladores de whitelist por asset feitos à mão
  em apps.

### Corrigido

- Apps recém-gerados herdam uma fixação de compatibilidade `time =
  0.3.47` no nível do framework, evitando conflitos de coerência do
  Rust 1.96 vindos de `time 0.3.48` em resoluções de dependência de
  scaffold recém-criado.

### Documentação

- Documentados os dois starter kits lançados - **Nebula** (auth nível
  Breeze) e **Pulsar** (site de produto + comunidade) - ao longo do
  manual, README e roadmap; roadmap reestruturado em torno da
  superfície lançada; e referências de versão reconciliadas ao longo
  da documentação.

## 0.1.0 - 2026-06-10

O release inicial do Suprnova. Suprnova é um framework web para Rust
inspirado no Laravel, feito como fork do Kit e levado numa direção
própria. O alvo de paridade de hoje é o Laravel 13.x.

Este release usa o modelo de distribuição por git: consumidores do
framework dependem de
`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`,
e a CLI se instala com `cargo install --git`.

### Adicionado

#### HTTP, roteamento e middleware

- `Router` com grupos de rota, prefixos, restrições de parâmetro, rotas
  nomeadas
- Registro de rota validado em tempo de compilação via a macro
  `routes!`
- Roteamento de recurso (`Router::resource`) produzindo as sete rotas
  padrão
- URLs assinadas (funções livres `url::signed_route` /
  `url::temporary_signed_route`, mais `Redirect::signed_route` /
  `Redirect::temporary_signed_route`)
- Helpers de redirect - `Redirect::to`, `Redirect::back`,
  `Redirect::route`, `Redirect::with_input`, `Redirect::with_errors`,
  `with_flash`
- Trait de middleware com camadas globais, de grupo e por rota
- Middleware embutido - CORS, CSRF, sessão, timeout de solicitação, ID
  de solicitação, throttle / throttle de login, verificação de URL
  assinada, autenticado, e-mail verificado, força bruta
- Helpers de abort (`abort`, `abort_unless`, `abort_if`)
- `suprnova::handle_request(...)` - adaptador público para servir uma
  única solicitação hyper contra um router + chain de middleware

#### Ponte de frontend Inertia.js

- `#[derive(InertiaProps)]` com emissão de tipos TypeScript
- Macro `inertia_response!` com validação de componente em tempo de
  compilação
- Três frontends starter de primeira classe - **Svelte 5** (com
  runes), **React 19**, **Vue 3.5** - todos sobre Inertia 3.1.1 + Vite
  8 + Tailwind v4
- Reloads parciais (`only` / `except`), props diferidas, layout
  persistente, histórico criptografado, preservação de scroll
- `Inertia::paginate(component, key, paginator)` para conexão de
  paginador → prop Inertia

#### ORM estilo Eloquent (sobre o SeaORM)

- Macro de atributo `#[suprnova::model]` que emite uma entity SeaORM e
  o struct Eloquent voltado ao usuário em uma só tacada
- Trait `Model` completa - `create`, `find`, `find_or_fail`,
  `find_many`, `all`, `query`, `save`, `update`, `delete`,
  `force_delete`, `refresh`, `fresh`, `replicate`, `replicate_into`,
  `increment`/`decrement`, `destroy`, `is`/`is_not`, `to_array`/`to_json`
- Mass-assignment fillable / guarded com envelope `Attrs`
- 22 casts de attribute - booleanos, inteiros, floats, datas, enums,
  hashed, encrypted, JSON, coleções, dinheiro, datetime com fuso
  horário
- Acessadores / mutadores via `#[suprnova::model]`
- Timestamps automáticos (`created_at`, `updated_at`)
- Soft deletes (`deleted_at`) com `force_delete`, `restore`,
  `trashed`, `only_trashed`, `with_trashed`
- Onze tipos de relação - `HasOne`, `HasMany`, `BelongsTo`,
  `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`,
  `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany`
- Enums de morph por família + registro de morph com rotação de
  `APP_KEY_PREVIOUS`
- Eager loading via `.with(...)`, `.with_count(...)`,
  `.load_missing(...)`
- Motor EXISTS correlacionado para `has` / `where_has`
- Dezesseis eventos de ciclo de vida (retrieving, retrieved, creating,
  created, updating, updated, saving, saved, deleting, deleted,
  restoring, restored, force-deleting, force-deleted, replicating,
  trashed)
- Trait `Observer<M>` com auto-registro por método via inventory
- Scopes locais via `#[scopes(M)]`, scopes globais via `GlobalScope`
- Superfície `Collection<M>` do Laravel - `pluck`, `key_by`,
  `group_by`, `where_in`, `first_where`, `contains_where`,
  `partition`, etc.
- Três paginadores - `paginate` (length-aware), `simple_paginate`,
  `cursor_paginate` - todos serializando para JSON no formato Laravel
- `chunk` / `lazy` / `cursor` para iteração de linhas em massa sem OOM
- Locking a nível de linha `lock_for_update` / `shared_lock`
- Construtor de consultas `DB::table(...)` com `DynamicRow` para
  queries ad-hoc
- `DB::transaction(...)` com savepoints, retry em deadlock, split de
  leitura/escrita multi-conexão
- `DB::listen(...)` + eventos `QueryExecuted` / `TransactionBegan` /
  `TransactionCommitted` / `TransactionRolledBack`
- Trait `Prunable` + comando de console `model:prune`
- Métodos helper de query `dump` / `dd`
- `#[model(unique_id="...")]` para chaves primárias UUID / ULID

#### Autenticação

- Trait `Authenticatable` + `EloquentUserProvider<M>`
- `Auth::attempt`, `Auth::login`, `Auth::user`, `Auth::user_or_fail`,
  `Auth::user_as<T>`, `Auth::logout`, `Auth::check`
- Múltiplos guards nomeados (sessão web, token de API)
- Fluxo de verificação de e-mail - `EmailVerification`,
  `EnsureEmailVerifiedMiddleware`, URLs de verificação assinadas,
  `EmailVerificationMail`
- Fluxo de redefinição de senha - `PasswordReset`, tokens com
  throttle, `PasswordChangedMail`, evento `PasswordResetLinkSent`
- TOTP de dois fatores - cadastro, verificação, códigos de
  recuperação, proteção contra replay
- Força bruta / throttle de login - chaveado por IP + identificador,
  `LoginThrottleMiddleware`
- Cookies remember-me com tokens opacos estáveis
- Seis eventos de auth - `LoginAttempted`, `LoggedIn`,
  `Authenticated`, `LoggedOut`, `PasswordResetLinkSent`,
  `EmailVerified`
- Sessões de navegador apoiadas no fork do Torii em
  `github.com/eas4ai/suprnova-torii-rs`

#### Autorização

- Facade `Gate` - `define`, `allows`, `denies`, `authorize`, `any`,
  `none`, `check` (variantes síncrona + assíncrona)
- Macro `#[policy(Model)]` para registro de policy
- Auto-autorização de rota de recurso

#### Pagamentos

- Superfície de cinco traits agnóstica a provider - `Checkout`,
  `Payment`, `Subscription`, `CustomerStore`, `WebhookHandler`
- Trait guarda-chuva `PaymentProvider` + consulta de capacidade via
  `as_payment()`
- Espelho no banco de dados - `customers`, `subscriptions`,
  `subscription_items`, `payments`, `refunds`,
  `payment_webhook_events` (UNIQUE para idempotência)
- Enum `SessionPayload` marcado por fluxo (avulso vs assinatura)
- Dois adaptadores de referência como crates do workspace -
  `suprnova-payments-stripe` (gateway, impl completa de `Payment`),
  `suprnova-payments-paddle` (Merchant of Record, sem impl de
  `Payment`)
- Provider mock para testes

#### Fila, jobs, batches, chains

- Trait `Job` - `handle`, `max_tries`, `backoff`, `timeout`,
  `fail_on_timeout`
- `Queue::push`, `Queue::push_later`, `Queue::push_unique`,
  `Queue::push_unique_later`
- Drivers - `sync`, `null`, `redis`, `database`
- Trait `JobMiddleware` - seis middleware embutidos
- Batches e chains - `Queue::batch(jobs).dispatch()`, construtor
  fluente de chain, cancelamento, rastreamento de progresso
- Armazenamento de failed-jobs com replay
- Worker com shutdown gracioso, concorrência configurável,
  recuperação de panic via `catch_unwind`, métricas de liquidação
- Doze eventos de fila cobrindo enfileiramento, processamento, falha,
  release, ciclo de vida do worker

#### Transmissão e WebSockets

- Macro `ws!()` + `Router::ws` para endpoints WebSocket tipados
- Split Sink/Stream de `WsSocket`
- Supervisors com auto-restart via trait `Supervisor`
- `BroadcastHub` com canais `Channel`, `Private`, `Presence`
- Protocolo de envelope JSON, presence join/leave/here, TTL de
  presence configurável com recuperação de crash
- Ponte `Broadcastable` para o `EventDispatcher`
- Heartbeat de close-on-no-pong com drain de WS_TASKS configurável
- Middleware de WebSocket por rota
- Padrões mais seguros de 1 MiB / 64 KiB + factory
  `WsConfig::generous()`
- Política de origem + close 1011 em violação de protocolo

#### Notificações e correio

- Trait `Notification` + `Notify::send(recipient, notification).await`
- Mailable + renderização de template Markdown
- Canais de banco de dados / mail / broadcast / web-push
- Assinatura VAPID + criptografia de payload ECE do RFC 8291 (via
  `suprnova-web-push`)
- Validação de subject VAPID, parsing de retry-after, cap de 8 KiB no
  corpo de rejeição
- Trait Notifiable para tipagem de destinatário

#### Eventos

- Dispatcher de evento tipado - `EventFacade::dispatch`,
  `EventFacade::listen<E, L>`, `EventFacade::forget`
- Eventos saving/updating canceláveis (retornam
  `EventResult::cancel`)
- Listeners enfileiráveis

#### Sistema de arquivos

- `Storage::disk("name")` com suporte multi-driver - local, S3, Azure,
  GCS via OpenDAL
- Mover, copiar, existência, tamanho, mime, última modificação,
  prepend/append
- Uploads e downloads via streaming

#### Cache

- `Cache::store("name")` + registro de driver
- Drivers - memory, redis (com connect-timeout limitado), database,
  file
- `remember`, `forever`, `tags`, incremento/decremento atômico, locks

#### Banco de dados vetorial

- Trait `VectorDriver` com quatro drivers - in-memory, Qdrant
  (mapeamento de ID via UUID-5), Pinecone (IDs string nativos),
  MariaDB nativo `VECTOR(N)` + índices HNSW (11.7+)
- Distância cosseno / produto interno / euclidiana

#### Binário do console e CLI

- Binário `console` por projeto - análogo Rust do `php artisan`, roda
  comandos definidos pelo usuário via `#[suprnova::console::command]`
- `#[derive(Command)]` para argumentos tipados
- CLI `suprnova` - `new`, `serve`, `migrate`, `db:sync`,
  `generate-types`, `key:generate`,
  `make:{controller,middleware,action,error,inertia,migration,task,command}`,
  `db:seed`, `model:prune`
- Flag `--version`
- Templates de scaffold para starters de backend + API nos três
  frontends

#### Sinalizadores de recursos

- `DatabaseEvaluator` com carregamento de snapshot
- `CachedEvaluator` com TTL
- Extractor `FeatureMiddleware`
- Superfície CRUD de admin
- Trait `FeatureSync` para propagação sub-segundo entre processos

#### Agendamento

- Parser de expressão cron
- `Schedule::task(...)` com predicados componíveis
- Locks de servidor único, prevenção de overlap, rastreamento de
  dispatch
- Comando de console `schedule:run`

#### Validação

- Integração com `validator` 0.20
- Macros `#[request]` + `#[derive(FormRequest)]`
- Cap de tamanho por formulário `#[form_request(max_body_bytes = N)]`
- `#[form_request(custom_hooks)]` para opt-out num `impl FormRequest`
  escrito pelo usuário
- Hooks de ciclo de vida - `authorize`, `after_validation`,
  `after_validation_async`

#### Drivers de banco de dados

- Suporte apoiado em SeaORM para SQLite, Postgres, MySQL, MariaDB
- Detecção de driver baseada em URL
- Sistema de migração + `migrate`, `migrate:rollback`,
  `migrate:status`, `migrate:fresh`, `migrate:refresh`

#### Cliente HTTP

- Facade `Http` - `get` / `post` / `put` / `patch` / `delete`
  retornando um `RequestBuilder`; `.send().await` produz um
  `ClientResponse`
- TLS rustls, timeout padrão de 30s, user-agent `suprnova/<version>`
- Métodos encadeáveis `json` / `form` / `body` / `header` /
  `bearer_token` / `basic_auth` / `timeout`
- `RequestBuilder::retry(max_attempts, base_backoff)` - backoff
  exponencial para falhas transientes e 5xx; respeita `Retry-After`
- Guarda de teste `Http::fake(|| async { ... }).await` com
  `fake_response(method, url_substring, status, body)` +
  `assert_sent` / `assert_not_sent`

#### Criptografia

- Facade estática `Crypt` + `EncryptionKey` (`crypto::*`);
  AES-256-GCM com nonces aleatórios de 12 bytes
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- Vinculação AAD `CryptPurpose` prevenindo replay cross-protocol
- Rotação de `APP_KEY_PREVIOUS`
- Comando de CLI `suprnova key:generate` para cunhar chaves novas

#### Testes

- Macro de teste assíncrono `#[suprnova_test]`
- `TestDatabase::fresh::<Migrator>()` com instâncias seguras para
  paralelismo
- `TestContainer::bind` para mocks por teste
- Helpers de teste HTTP - `Test::get`, `Test::post`, JSON / form /
  multipart
- Fakes de Queue / Mail / Notification / Event
- `assert_emitted`, `assert_dispatched`, `assert_dispatched_times`

### Alterado

- Os fluxos de verificação de auth e redefinição de senha agora operam
  através do provedor de usuário configurado, em vez de internals do
  Torii.
- Apps gerados precisam implementar `get_auth_password`; exemplos com
  scaffold agora falham de forma explícita, em vez de deixar o login
  sempre falhar silenciosamente.
- O gate de release local está conectado em `scripts/release.sh`, e o
  repositório inclui um hook pre-push obrigatório para fmt, clippy,
  testes, docs e builds de feature.
- A documentação de porta de dev com scaffold se mudou para os
  padrões atuais de backend/frontend (`8765` / `5765`), com `dev:tls`
  e `--with-portless` documentados.
- `MAIL_FROM` é validado antes de tokens de verificação ou redefinição
  serem emitidos, evitando linhas órfãs de auth-flow quando a
  configuração de mail é inválida.

### Corrigido

- Drift do template de scaffold do React em relação ao starter
  lançado.
- Grupos de rota raiz não geram mais paths `//` duplicados.
- Redirects de path literal agora despacham pelo caminho de
  roteamento pretendido.
- Testes de fanout de transmissão agora tratam resultados de `track`
  / `untrack`.
- O driver de log de mail emite o corpo de texto renderizado, então
  links de verificação e redefinição de senha aparecem nos logs de
  desenvolvimento local.
- A cobertura de redefinição de senha fixa o comportamento de
  revogação de sessão e remember-me.

### Notas

- **Modelo de distribuição**: baseado em git de ponta a ponta.
  `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`;
  CLI via `cargo install --git`. Nada é publicado no crates.io.
