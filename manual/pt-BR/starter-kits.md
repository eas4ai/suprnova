# Kits iniciais

Kits iniciais são aplicações Suprnova prontas para uso que você faz fork e
publica. Cada um conecta controladores, rotas, migrações, páginas de frontend
e testes para uma superfície de produto completa - para que você comece com um
app funcionando, não com um scaffold vazio.

Dois kits estão disponíveis hoje, modelados na linhagem do Laravel. Escolha o
mais próximo do que você está construindo e customize a partir daí.

## Nebula - autenticação (nível Breeze)

**Repositório: [github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

O kit de autenticação completa mínimo - equivalente Breeze de Suprnova. Tudo o
que você precisa para contas e nada do que não precisa:

- Registro com verificação de email
- Login com "lembrar-me"
- Redefinição de senha com respostas anti-enumeração
- Gerenciamento de perfil - atualizar email e senha, deletar conta
- Um frontend Inertia 3 + Svelte 5 com marca própria (escuro por padrão), com
  o menu de usuário logado conectado

Nebula inclui dois suites de testes: lógica de auth em nível de facade, e um
suite HTTP de nível de wire que percorre rotas reais, sessões, trocas CSRF, e
gates guest / auth / verified sobre um socket loopback.

Use Nebula quando você quer uma fundação limpa de gerenciamento de conta para
construir seu próprio produto por cima.

## Pulsar - site de produto e comunidade

**Repositório: [github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

Um site completo de ferramenta para desenvolvedores / SaaS em Vue 3.5 + Vuetify.
Tudo na história de autenticação do Nebula, mais as superfícies que um site de
produto real precisa:

- Página de landing de marketing e um dashboard do usuário
- Um pipeline de documentação Markdown (`docs:build`) com busca e um índice
  gerado automaticamente
- Um sistema de blog / artigos com feed RSS
- Perfis públicos de membros
- Taxonomia - tópicos, tags e categorias
- Controle de acesso baseado em papéis: roles, permissions, e gates
- Superfícies de admin e moderação para conteúdo e membros

Pulsar é o kit de origem para produtos downstream como `suprnova.app`. Use
Pulsar quando você está lançando um site de produto com docs, um blog e uma
comunidade de membros - não apenas autenticação.

## Qual kit?

| Você quer… | Comece com |
|---|---|
| Contas e um lugar para construir | **Nebula** |
| Um site de produto completo - landing, docs, blog, comunidade, RBAC | **Pulsar** |
| Um backend só API (auth por token, sem frontend) | `suprnova new my-api --api` |

Ambos os kits rastreiam o framework como uma dependência git e rodam na mesma
stack que você já conhece - veja o README de cada repositório para setup. Mais
kits estão planejados; acompanhe os
[releases](https://github.com/eas4ai/suprnova/releases) ou abra uma
issue se houver um que você queira.

## O que o scaffold padrão oferece

Se nenhum kit se encaixa, `suprnova new my-app --frontend svelte` (ou `react`,
ou `vue`) já inclui um fluxo de autenticação funcionando - login, registro,
logout, autenticação de sessão com middleware `authenticate`, proteção CSRF, e
uma rota `/dashboard` protegida - em qualquer um dos três frontends (Svelte 5,
React 19, Vue 3.5) com Tailwind v4 e Inertia v3. Veja
[Instalação](installation.md) para a saída do scaffold e
[Início rápido](quickstart.md) para o passo a passo dos primeiros cinco minutos.

Para serviços somente API, `suprnova new my-api --api` inicializa o Magnetar,
instala middleware de sessão bearer e gera o scaffold de registro e login por
senha contra a tabela canônica `app_users`, sem frontend.

## Contribuindo com um kit inicial

Você construiu algo reutilizável em cima de Suprnova e quer fazer upstream como
um kit canônico? Veja [Contribuição](contributions.md). Ficamos felizes em pegar
uma implementação real e transformá-la em um kit genérico.
