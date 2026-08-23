SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: app_users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.app_users (
    id bigint NOT NULL,
    email character varying NOT NULL,
    name character varying,
    password_hash character varying,
    remember_token character varying,
    email_verified_at timestamp with time zone,
    locked_at timestamp with time zone,
    auth_epoch bigint NOT NULL,
    created_at timestamp with time zone,
    updated_at timestamp with time zone
);


--
-- Name: app_users_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.app_users_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: app_users_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.app_users_id_seq OWNED BY public.app_users.id;


--
-- Name: auth_ceremonies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_ceremonies (
    id bigint NOT NULL,
    kind character varying NOT NULL,
    selector character varying NOT NULL,
    payload bytea NOT NULL,
    state character varying NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone
);


--
-- Name: auth_ceremonies_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.auth_ceremonies_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: auth_ceremonies_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.auth_ceremonies_id_seq OWNED BY public.auth_ceremonies.id;


--
-- Name: auth_lifecycle_deliveries; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_lifecycle_deliveries (
    mutation_id character varying NOT NULL,
    lease_id character varying,
    lease_until timestamp with time zone,
    delivered_at timestamp with time zone
);


--
-- Name: auth_linked_accounts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_linked_accounts (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    provider character varying NOT NULL,
    provider_account_id character varying NOT NULL,
    created_at timestamp with time zone,
    updated_at timestamp with time zone
);


--
-- Name: auth_linked_accounts_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.auth_linked_accounts_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: auth_linked_accounts_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.auth_linked_accounts_id_seq OWNED BY public.auth_linked_accounts.id;


--
-- Name: auth_lockouts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_lockouts (
    id bigint NOT NULL,
    identity character varying NOT NULL,
    attempted_at timestamp with time zone NOT NULL,
    ip_address character varying,
    migration_source_id character varying,
    locked_at timestamp with time zone,
    reason character varying
);


--
-- Name: auth_lockouts_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.auth_lockouts_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: auth_lockouts_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.auth_lockouts_id_seq OWNED BY public.auth_lockouts.id;


--
-- Name: auth_methods; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_methods (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    credential_id character varying,
    public_key character varying,
    created_at timestamp with time zone
);


--
-- Name: auth_methods_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.auth_methods_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: auth_methods_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.auth_methods_id_seq OWNED BY public.auth_methods.id;


--
-- Name: auth_migration_identities; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_migration_identities (
    id character varying NOT NULL,
    plan_id character varying NOT NULL,
    source_user_id character varying NOT NULL,
    app_user_id bigint NOT NULL
);


--
-- Name: auth_migration_runs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_migration_runs (
    plan_id character varying NOT NULL,
    imports_committed boolean NOT NULL,
    completed_at timestamp with time zone
);


--
-- Name: auth_provider_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_provider_tokens (
    id character varying NOT NULL,
    provider character varying NOT NULL,
    access_ciphertext bytea NOT NULL,
    refresh_ciphertext bytea,
    raw_payload_ciphertext bytea NOT NULL,
    token_type character varying NOT NULL,
    scopes character varying NOT NULL,
    access_expires_at timestamp with time zone,
    generation bigint NOT NULL,
    claim_id character varying,
    claim_deadline timestamp with time zone,
    revoked_at timestamp with time zone,
    revoked_reused boolean,
    created_at timestamp with time zone NOT NULL
);


--
-- Name: auth_remember_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_remember_tokens (
    id character varying NOT NULL,
    selector character varying NOT NULL,
    user_id character varying NOT NULL,
    auth_epoch bigint NOT NULL,
    verifier_hash character varying NOT NULL,
    expires_at timestamp with time zone NOT NULL
);


--
-- Name: auth_sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_sessions (
    id character varying NOT NULL,
    user_id bigint NOT NULL,
    auth_epoch bigint NOT NULL,
    token_digest character varying NOT NULL,
    token_hash character varying,
    user_agent character varying,
    ip_address character varying,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone
);


--
-- Name: auth_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_tokens (
    id bigint NOT NULL,
    user_id bigint,
    purpose character varying NOT NULL,
    digest character varying NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    created_at timestamp with time zone,
    updated_at timestamp with time zone
);


--
-- Name: auth_tokens_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.auth_tokens_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: auth_tokens_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.auth_tokens_id_seq OWNED BY public.auth_tokens.id;


--
-- Name: auth_two_factor; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_two_factor (
    user_id character varying NOT NULL,
    secret bytea NOT NULL,
    recovery_codes bytea,
    enrollment_auth_epoch bigint NOT NULL,
    enrollment_session_id character varying,
    enrollment_expires_at timestamp with time zone,
    rotation_pending boolean NOT NULL,
    confirmed_at timestamp with time zone,
    last_used_timestep bigint,
    created_at timestamp with time zone,
    updated_at timestamp with time zone
);


--
-- Name: magnetar_migration_state; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.magnetar_migration_state (
    key character varying NOT NULL,
    value character varying NOT NULL
);


--
-- Name: app_users id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_users ALTER COLUMN id SET DEFAULT nextval('public.app_users_id_seq'::regclass);


--
-- Name: auth_ceremonies id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_ceremonies ALTER COLUMN id SET DEFAULT nextval('public.auth_ceremonies_id_seq'::regclass);


--
-- Name: auth_linked_accounts id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_linked_accounts ALTER COLUMN id SET DEFAULT nextval('public.auth_linked_accounts_id_seq'::regclass);


--
-- Name: auth_lockouts id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_lockouts ALTER COLUMN id SET DEFAULT nextval('public.auth_lockouts_id_seq'::regclass);


--
-- Name: auth_methods id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_methods ALTER COLUMN id SET DEFAULT nextval('public.auth_methods_id_seq'::regclass);


--
-- Name: auth_tokens id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_tokens ALTER COLUMN id SET DEFAULT nextval('public.auth_tokens_id_seq'::regclass);


--
-- Name: app_users app_users_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_users
    ADD CONSTRAINT app_users_pkey PRIMARY KEY (id);


--
-- Name: auth_ceremonies auth_ceremonies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_ceremonies
    ADD CONSTRAINT auth_ceremonies_pkey PRIMARY KEY (id);


--
-- Name: auth_lifecycle_deliveries auth_lifecycle_deliveries_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_lifecycle_deliveries
    ADD CONSTRAINT auth_lifecycle_deliveries_pkey PRIMARY KEY (mutation_id);


--
-- Name: auth_linked_accounts auth_linked_accounts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_linked_accounts
    ADD CONSTRAINT auth_linked_accounts_pkey PRIMARY KEY (id);


--
-- Name: auth_lockouts auth_lockouts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_lockouts
    ADD CONSTRAINT auth_lockouts_pkey PRIMARY KEY (id);


--
-- Name: auth_methods auth_methods_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_methods
    ADD CONSTRAINT auth_methods_pkey PRIMARY KEY (id);


--
-- Name: auth_migration_identities auth_migration_identities_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_migration_identities
    ADD CONSTRAINT auth_migration_identities_pkey PRIMARY KEY (id);


--
-- Name: auth_migration_runs auth_migration_runs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_migration_runs
    ADD CONSTRAINT auth_migration_runs_pkey PRIMARY KEY (plan_id);


--
-- Name: auth_provider_tokens auth_provider_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_provider_tokens
    ADD CONSTRAINT auth_provider_tokens_pkey PRIMARY KEY (id);


--
-- Name: auth_remember_tokens auth_remember_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_remember_tokens
    ADD CONSTRAINT auth_remember_tokens_pkey PRIMARY KEY (id);


--
-- Name: auth_sessions auth_sessions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_sessions
    ADD CONSTRAINT auth_sessions_pkey PRIMARY KEY (id);


--
-- Name: auth_tokens auth_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_tokens
    ADD CONSTRAINT auth_tokens_pkey PRIMARY KEY (id);


--
-- Name: auth_two_factor auth_two_factor_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_two_factor
    ADD CONSTRAINT auth_two_factor_pkey PRIMARY KEY (user_id);


--
-- Name: magnetar_migration_state magnetar_migration_state_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.magnetar_migration_state
    ADD CONSTRAINT magnetar_migration_state_pkey PRIMARY KEY (key);


--
-- Name: auth_linked_accounts_provider_subject; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX auth_linked_accounts_provider_subject ON public.auth_linked_accounts USING btree (provider, provider_account_id);


--
-- Name: auth_lockouts_migration_source; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX auth_lockouts_migration_source ON public.auth_lockouts USING btree (migration_source_id);



SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Data for Name: app_users; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.app_users (id, email, name, password_hash, remember_token, email_verified_at, locked_at, auth_epoch, created_at, updated_at) VALUES (6100, 'seaorm11@example.test', 'SeaORM 1.1 fixture', 'fixture-hash', NULL, '2026-08-23 12:00:00+00', NULL, 0, '2026-08-23 12:00:00+00', '2026-08-23 12:00:00+00');


--
-- Data for Name: auth_ceremonies; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_lifecycle_deliveries; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_linked_accounts; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_lockouts; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_methods; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_migration_identities; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_migration_runs; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_provider_tokens; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_remember_tokens; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_sessions; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.auth_sessions (id, user_id, auth_epoch, token_digest, token_hash, user_agent, ip_address, expires_at, revoked_at) VALUES ('seaorm11-session', 6100, 0, '3abda255ff0ff2f5f526a8898ca27e01554f300be8641bbf15df18415e8ae9b1', '3abda255ff0ff2f5f526a8898ca27e01554f300be8641bbf15df18415e8ae9b1', 'fixture-agent', '192.0.2.61', '2036-08-23 12:00:00+00', NULL);


--
-- Data for Name: auth_tokens; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.auth_tokens (id, user_id, purpose, digest, expires_at, used_at, created_at, updated_at) VALUES (6102, 6100, 'password-reset', '102ce6cc88841311d1339a55bdea7604675bc6a864be3ad7ca4f955a3fad83c0', '2036-08-23 12:00:00+00', NULL, '2026-08-23 12:00:00+00', '2026-08-23 12:00:00+00');


--
-- Data for Name: auth_two_factor; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: magnetar_migration_state; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.magnetar_migration_state (key, value) VALUES ('schema_version', '1');


--
-- Name: app_users_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.app_users_id_seq', 1, false);


--
-- Name: auth_ceremonies_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.auth_ceremonies_id_seq', 1, false);


--
-- Name: auth_linked_accounts_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.auth_linked_accounts_id_seq', 1, false);


--
-- Name: auth_lockouts_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.auth_lockouts_id_seq', 1, false);


--
-- Name: auth_methods_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.auth_methods_id_seq', 1, false);


--
-- Name: auth_tokens_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.auth_tokens_id_seq', 1, false);
