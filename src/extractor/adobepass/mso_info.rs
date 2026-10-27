//! Upstream `MSO_INFO` (`yt_dlp/extractor/adobepass.py`), generated from
//! the pinned upstream checkout (f264a6cbd1f630894da0f368df00f8aae50140e0)
//! by `tools/generate_adobepass_mso_info.py`; do not edit by hand.
use super::MsoInfo;

/// The TV providers by Adobe Pass MSO id, in upstream order.
pub(crate) const MSO_INFO: &[(&str, MsoInfo)] = &[
    (
        "DTV",
        MsoInfo {
            name: "DIRECTV",
            username_field: Some("username"),
            password_field: Some("password"),
            login_hostname: None,
        },
    ),
    (
        "ATT",
        MsoInfo {
            name: "AT&T U-verse",
            username_field: Some("userid"),
            password_field: Some("password"),
            login_hostname: None,
        },
    ),
    (
        "ATTOTT",
        MsoInfo {
            name: "DIRECTV NOW",
            username_field: Some("email"),
            password_field: Some("loginpassword"),
            login_hostname: None,
        },
    ),
    (
        "RCN",
        MsoInfo {
            name: "RCN",
            username_field: Some("username"),
            password_field: Some("password"),
            login_hostname: None,
        },
    ),
    (
        "Rogers",
        MsoInfo {
            name: "Rogers",
            username_field: Some("UserName"),
            password_field: Some("UserPassword"),
            login_hostname: None,
        },
    ),
    (
        "Comcast_SSO",
        MsoInfo {
            name: "Comcast XFINITY",
            username_field: Some("user"),
            password_field: Some("passwd"),
            login_hostname: Some("login.xfinity.com"),
        },
    ),
    (
        "TWC",
        MsoInfo {
            name: "Time Warner Cable | Spectrum",
            username_field: Some("Ecom_User_ID"),
            password_field: Some("Ecom_Password"),
            login_hostname: None,
        },
    ),
    (
        "Brighthouse",
        MsoInfo {
            name: "Bright House Networks | Spectrum",
            username_field: Some("j_username"),
            password_field: Some("j_password"),
            login_hostname: None,
        },
    ),
    (
        "Charter_Direct",
        MsoInfo {
            name: "Charter Spectrum",
            username_field: Some("IDToken1"),
            password_field: Some("IDToken2"),
            login_hostname: None,
        },
    ),
    (
        "Spectrum",
        MsoInfo {
            name: "Spectrum",
            username_field: Some("IDToken1"),
            password_field: Some("IDToken2"),
            login_hostname: None,
        },
    ),
    (
        "Philo",
        MsoInfo {
            name: "Philo",
            username_field: Some("ident"),
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "Verizon",
        MsoInfo {
            name: "Verizon FiOS",
            username_field: Some("IDToken1"),
            password_field: Some("IDToken2"),
            login_hostname: Some("ssoauth.verizon.com"),
        },
    ),
    (
        "Fubo",
        MsoInfo {
            name: "Fubo",
            username_field: Some("username"),
            password_field: Some("password"),
            login_hostname: None,
        },
    ),
    (
        "Cablevision",
        MsoInfo {
            name: "Optimum/Cablevision",
            username_field: Some("j_username"),
            password_field: Some("j_password"),
            login_hostname: None,
        },
    ),
    (
        "thr030",
        MsoInfo {
            name: "3 Rivers Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com140",
        MsoInfo {
            name: "Access Montana",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "acecommunications",
        MsoInfo {
            name: "AcenTek",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "acm010",
        MsoInfo {
            name: "Acme Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ada020",
        MsoInfo {
            name: "Adams Cable Service",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "alb020",
        MsoInfo {
            name: "Albany Mutual Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "algona",
        MsoInfo {
            name: "Algona Municipal Utilities",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "allwest",
        MsoInfo {
            name: "All West Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "all025",
        MsoInfo {
            name: "Allen's Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "spl010",
        MsoInfo {
            name: "Alliance Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "all070",
        MsoInfo {
            name: "ALLO Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "alpine",
        MsoInfo {
            name: "Alpine Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hun015",
        MsoInfo {
            name: "American Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nwc010",
        MsoInfo {
            name: "American Broadband Missouri",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com130-02",
        MsoInfo {
            name: "American Community Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com130-01",
        MsoInfo {
            name: "American Warrior Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tom020",
        MsoInfo {
            name: "Amherst Telephone/Tomorrow Valley",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tvc020",
        MsoInfo {
            name: "Andycable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "arkwest",
        MsoInfo {
            name: "Arkwest Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "art030",
        MsoInfo {
            name: "Arthur Mutual Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "arvig",
        MsoInfo {
            name: "Arvig",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcash010",
        MsoInfo {
            name: "Ashland Home Net",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "astound",
        MsoInfo {
            name: "Astound (now Wave)",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dix030",
        MsoInfo {
            name: "ATC Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ara010",
        MsoInfo {
            name: "ATC Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "she030-02",
        MsoInfo {
            name: "Ayersville Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "baldwin",
        MsoInfo {
            name: "Baldwin Lightstream",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bal040",
        MsoInfo {
            name: "Ballard TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit025",
        MsoInfo {
            name: "Bardstown Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bay030",
        MsoInfo {
            name: "Bay Country Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel095",
        MsoInfo {
            name: "Beaver Creek Cooperative Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bea020",
        MsoInfo {
            name: "Beaver Valley Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bee010",
        MsoInfo {
            name: "Bee Line Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wir030",
        MsoInfo {
            name: "Beehive Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bra020",
        MsoInfo {
            name: "BELD",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bel020",
        MsoInfo {
            name: "Bellevue Municipal Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "vol040-01",
        MsoInfo {
            name: "Ben Lomand Connect / BLTV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bev010",
        MsoInfo {
            name: "BEVCOMM",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "big020",
        MsoInfo {
            name: "Big Sandy Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ble020",
        MsoInfo {
            name: "Bledsoe Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bvt010",
        MsoInfo {
            name: "Blue Valley Tele-Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bra050",
        MsoInfo {
            name: "Brandenburg Telephone Co.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bte010",
        MsoInfo {
            name: "Bristol Tennessee Essential Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "annearundel",
        MsoInfo {
            name: "Broadstripe",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "btc010",
        MsoInfo {
            name: "BTC Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "btc040",
        MsoInfo {
            name: "BTC Vision - Nahunta",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bul010",
        MsoInfo {
            name: "Bulloch Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "but010",
        MsoInfo {
            name: "Butler-Bremer Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel160-csp",
        MsoInfo {
            name: "C Spire SNAP",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "csicable",
        MsoInfo {
            name: "Cable Services Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cableamerica",
        MsoInfo {
            name: "CableAmerica",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cab038",
        MsoInfo {
            name: "CableSouth Media 3",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-camtel",
        MsoInfo {
            name: "Cam-Tel Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "car030",
        MsoInfo {
            name: "Cameron Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "canbytel",
        MsoInfo {
            name: "Canby Telcom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "crt020",
        MsoInfo {
            name: "CapRock Tv",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "car050",
        MsoInfo {
            name: "Carnegie Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cas",
        MsoInfo {
            name: "CAS Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "casscomm",
        MsoInfo {
            name: "CASSCOMM",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid180-02",
        MsoInfo {
            name: "Catalina Broadband Solutions",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cccomm",
        MsoInfo {
            name: "CC Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttccde010",
        MsoInfo {
            name: "CDE Lightband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cfunet",
        MsoInfo {
            name: "Cedar Falls Utilities",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-01",
        MsoInfo {
            name: "Celect-Bloomer Telephone Area",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-02",
        MsoInfo {
            name: "Celect-Bruce Telephone Area",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-03",
        MsoInfo {
            name: "Celect-Citizens Connected Area",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-04",
        MsoInfo {
            name: "Celect-Elmwood/Spring Valley Area",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-06",
        MsoInfo {
            name: "Celect-Mosaic Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dem010-05",
        MsoInfo {
            name: "Celect-West WI Telephone Area",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "net010-02",
        MsoInfo {
            name: "Cellcom/Nsight Telservices",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cen100",
        MsoInfo {
            name: "CentraCom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttccst010",
        MsoInfo {
            name: "Central Scott / CSTV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cha035",
        MsoInfo {
            name: "Chaparral CableVision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cha050",
        MsoInfo {
            name: "Chariton Valley Communication Corporation, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cha060",
        MsoInfo {
            name: "Chatmoss Cablevision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcche010",
        MsoInfo {
            name: "Cherokee Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "che050",
        MsoInfo {
            name: "Chesapeake Bay Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cimtel",
        MsoInfo {
            name: "Cim-Tel Cable, LLC.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit180",
        MsoInfo {
            name: "Citizens Cablevision - Floyd, VA",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit210",
        MsoInfo {
            name: "Citizens Cablevision, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit040",
        MsoInfo {
            name: "Citizens Fiber",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit250",
        MsoInfo {
            name: "Citizens Mutual",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "war040",
        MsoInfo {
            name: "Citizens Telephone Corporation",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wat025",
        MsoInfo {
            name: "City Of Monroe",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wadsworth",
        MsoInfo {
            name: "CityLink",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor100",
        MsoInfo {
            name: "CL Tel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cla010",
        MsoInfo {
            name: "Clarence Telephone and Cedar Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ser060",
        MsoInfo {
            name: "Clear Choice Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tac020",
        MsoInfo {
            name: "Click! Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "war020",
        MsoInfo {
            name: "CLICK1.NET",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cml010",
        MsoInfo {
            name: "CML Telephone Cooperative Association",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cns",
        MsoInfo {
            name: "CNS",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com160",
        MsoInfo {
            name: "Co-Mo Connect",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "coa020",
        MsoInfo {
            name: "Coast Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "coa030",
        MsoInfo {
            name: "Coaxial Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid055",
        MsoInfo {
            name: "Cobalt TV (Mid-State Community TV)",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "col070",
        MsoInfo {
            name: "Columbia Power & Water Systems",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "col080",
        MsoInfo {
            name: "Columbus Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor105",
        MsoInfo {
            name: "Communications 1 Cablevision, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com150",
        MsoInfo {
            name: "Community Cable & Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com020",
        MsoInfo {
            name: "Community Communications Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "coy010",
        MsoInfo {
            name: "commZoom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com025",
        MsoInfo {
            name: "Complete Communication Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cat020",
        MsoInfo {
            name: "Comporium",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com071",
        MsoInfo {
            name: "ComSouth Telesys",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "consolidatedcable",
        MsoInfo {
            name: "Consolidated",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "conwaycorp",
        MsoInfo {
            name: "Conway Corporation",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "coo050",
        MsoInfo {
            name: "Coon Valley Telecommunications Inc",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "coo080",
        MsoInfo {
            name: "Cooperative Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cpt010",
        MsoInfo {
            name: "CP-TEL",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cra010",
        MsoInfo {
            name: "Craw-Kan Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "crestview",
        MsoInfo {
            name: "Crestview Cable Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cross",
        MsoInfo {
            name: "Cross TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cro030",
        MsoInfo {
            name: "Crosslake Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ctc040",
        MsoInfo {
            name: "CTC - Brainerd MN",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "phe030",
        MsoInfo {
            name: "CTV-Beam - East Alabama",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cun010",
        MsoInfo {
            name: "Cunningham Telephone & Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dpc010",
        MsoInfo {
            name: "D & P Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dak030",
        MsoInfo {
            name: "Dakota Central Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcdel010",
        MsoInfo {
            name: "Delcambre Telephone LLC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel160-del",
        MsoInfo {
            name: "Delta Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sal040",
        MsoInfo {
            name: "DiamondNet",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ind060-dc",
        MsoInfo {
            name: "Direct Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "doy010",
        MsoInfo {
            name: "Doylestown Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dic010",
        MsoInfo {
            name: "DRN",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dtc020",
        MsoInfo {
            name: "DTC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dtc010",
        MsoInfo {
            name: "DTC Cable (Delhi)",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dum010",
        MsoInfo {
            name: "Dumont Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dun010",
        MsoInfo {
            name: "Dunkerton Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cci010",
        MsoInfo {
            name: "Duo County Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "eagle",
        MsoInfo {
            name: "Eagle Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-east",
        MsoInfo {
            name: "East Arkansas Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "eatel",
        MsoInfo {
            name: "EATEL Video, LLC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ell010",
        MsoInfo {
            name: "ECTA",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "emerytelcom",
        MsoInfo {
            name: "Emery Telcom Video LLC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor200",
        MsoInfo {
            name: "Empire Access",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "endeavor",
        MsoInfo {
            name: "Endeavor Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sun045",
        MsoInfo {
            name: "Enhanced Telecommunications Corporation",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid030",
        MsoInfo {
            name: "enTouch",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "epb020",
        MsoInfo {
            name: "EPB Smartnet",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "jea010",
        MsoInfo {
            name: "EPlus Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com065",
        MsoInfo {
            name: "ETC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ete010",
        MsoInfo {
            name: "Etex Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fbc-tele",
        MsoInfo {
            name: "F&B Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fal010",
        MsoInfo {
            name: "Falcon Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fam010",
        MsoInfo {
            name: "FamilyView CableVision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "far020",
        MsoInfo {
            name: "Farmers Mutual Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fay010",
        MsoInfo {
            name: "Fayetteville Public Utilities",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sal060",
        MsoInfo {
            name: "fibrant",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fid010",
        MsoInfo {
            name: "Fidelity Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "for030",
        MsoInfo {
            name: "FJ Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fli020",
        MsoInfo {
            name: "Flint River Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "far030",
        MsoInfo {
            name: "FMT - Jesup",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "foo010",
        MsoInfo {
            name: "Foothills Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "for080",
        MsoInfo {
            name: "Forsyth CableNet",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fbcomm",
        MsoInfo {
            name: "Frankfort Plant Board",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel160-fra",
        MsoInfo {
            name: "Franklin Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcftc010",
        MsoInfo {
            name: "FTC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fullchannel",
        MsoInfo {
            name: "Full Channel, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gar040",
        MsoInfo {
            name: "Gardonville Cooperative Telephone Association",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gbt010",
        MsoInfo {
            name: "GBT Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tec010",
        MsoInfo {
            name: "Genuine Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "clr010",
        MsoInfo {
            name: "Giant Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gla010",
        MsoInfo {
            name: "Glasgow EPB",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gle010",
        MsoInfo {
            name: "Glenwood Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gra060",
        MsoInfo {
            name: "GLW Broadband Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "goldenwest",
        MsoInfo {
            name: "Golden West Cablevision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "vis030",
        MsoInfo {
            name: "Grantsburg Telcom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gpcom",
        MsoInfo {
            name: "Great Plains Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "gri010",
        MsoInfo {
            name: "Gridley Cable Inc",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hbc010",
        MsoInfo {
            name: "H&B Cable Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hae010",
        MsoInfo {
            name: "Haefele TV Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "htc010",
        MsoInfo {
            name: "Halstad Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "har005",
        MsoInfo {
            name: "Harlan Municipal Utilities",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "har020",
        MsoInfo {
            name: "Hart Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ced010",
        MsoInfo {
            name: "Hartelco TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hea040",
        MsoInfo {
            name: "Heart of Iowa Communications Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "htc020",
        MsoInfo {
            name: "Hickory Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttchig010",
        MsoInfo {
            name: "Highland Communication Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hig030",
        MsoInfo {
            name: "Highland Media",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "spc010",
        MsoInfo {
            name: "Hilliary Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hin020",
        MsoInfo {
            name: "Hinton CATV Co.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hometel",
        MsoInfo {
            name: "HomeTel Entertainment, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hoodcanal",
        MsoInfo {
            name: "Hood Canal Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-hope",
        MsoInfo {
            name: "Hope - Prescott Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "horizoncable",
        MsoInfo {
            name: "Horizon Cable TV, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hor040",
        MsoInfo {
            name: "Horizon Chillicothe Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "htc030",
        MsoInfo {
            name: "HTC Communications Co. - IL",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "htccomm",
        MsoInfo {
            name: "HTC Communications, Inc. - IA",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wal005",
        MsoInfo {
            name: "Huxley Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "imon",
        MsoInfo {
            name: "ImOn Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ind040",
        MsoInfo {
            name: "Independence Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rrc010",
        MsoInfo {
            name: "Inland Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "stc020",
        MsoInfo {
            name: "Innovative Cable TV St Croix",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "car100",
        MsoInfo {
            name: "Innovative Cable TV St Thomas-St John",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "icc010",
        MsoInfo {
            name: "Inside Connect Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "int100",
        MsoInfo {
            name: "Integra Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "int050",
        MsoInfo {
            name: "Interstate Telecommunications Coop",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "irv010",
        MsoInfo {
            name: "Irvine Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "k2c010",
        MsoInfo {
            name: "K2 Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "kal010",
        MsoInfo {
            name: "Kalida Telephone Company, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "kal030",
        MsoInfo {
            name: "Kalona Cooperative Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "kmt010",
        MsoInfo {
            name: "KMTelecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "kpu010",
        MsoInfo {
            name: "KPU Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "kuh010",
        MsoInfo {
            name: "Kuhn Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lak130",
        MsoInfo {
            name: "Lakeland Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lan010",
        MsoInfo {
            name: "Langco",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lau020",
        MsoInfo {
            name: "Laurel Highland Total Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "leh010",
        MsoInfo {
            name: "Lehigh Valley Cooperative Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bra010",
        MsoInfo {
            name: "Limestone Cable/Bracken Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "loc020",
        MsoInfo {
            name: "LISCO",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lit020",
        MsoInfo {
            name: "Litestream",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel140",
        MsoInfo {
            name: "LivCom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "loc010",
        MsoInfo {
            name: "LocalTel Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-longview",
        MsoInfo {
            name: "Longview - Kilgore Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lon030",
        MsoInfo {
            name: "Lonsdale Video Ventures, LLC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lns010",
        MsoInfo {
            name: "Lost Nation-Elwood Telephone Co.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttclpc010",
        MsoInfo {
            name: "LPC Connect",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "lumos",
        MsoInfo {
            name: "Lumos Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "madison",
        MsoInfo {
            name: "Madison Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mad030",
        MsoInfo {
            name: "Madison County Cable Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcmah010",
        MsoInfo {
            name: "Mahaska Communication Group",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mar010",
        MsoInfo {
            name: "Marne & Elk Horn Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mcc040",
        MsoInfo {
            name: "McClure Telephone Co.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mctv",
        MsoInfo {
            name: "MCTV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "merrimac",
        MsoInfo {
            name: "Merrimac Communications Ltd.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "metronet",
        MsoInfo {
            name: "Metronet",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mhtc",
        MsoInfo {
            name: "MHTC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "midhudson",
        MsoInfo {
            name: "Mid-Hudson Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "midrivers",
        MsoInfo {
            name: "Mid-Rivers Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid045",
        MsoInfo {
            name: "Midstate Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mil080",
        MsoInfo {
            name: "Milford Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "min030",
        MsoInfo {
            name: "MINET",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcmin010",
        MsoInfo {
            name: "Minford TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "san040-02",
        MsoInfo {
            name: "Mitchell Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mlg010",
        MsoInfo {
            name: "MLGC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mon060",
        MsoInfo {
            name: "Mon-Cre TVE",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mou110",
        MsoInfo {
            name: "Mountain Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mou050",
        MsoInfo {
            name: "Mountain Village Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mtacomm",
        MsoInfo {
            name: "MTA Communications, LLC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mtc010",
        MsoInfo {
            name: "MTC Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "med040",
        MsoInfo {
            name: "MTC Technologies",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "man060",
        MsoInfo {
            name: "MTCC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mtc030",
        MsoInfo {
            name: "MTCO Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mul050",
        MsoInfo {
            name: "Mulberry Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mur010",
        MsoInfo {
            name: "Murray Electric System",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "musfiber",
        MsoInfo {
            name: "MUS FiberNET",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mpw",
        MsoInfo {
            name: "Muscatine Power & Water",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcsli010",
        MsoInfo {
            name: "myEVTV.com",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor115",
        MsoInfo {
            name: "NCC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor260",
        MsoInfo {
            name: "NDTC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nctc",
        MsoInfo {
            name: "Nebraska Central Telecom, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nel020",
        MsoInfo {
            name: "Nelsonville TV Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nem010",
        MsoInfo {
            name: "Nemont",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "new075",
        MsoInfo {
            name: "New Hope Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor240",
        MsoInfo {
            name: "NICP",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cic010",
        MsoInfo {
            name: "NineStar Connect",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nktelco",
        MsoInfo {
            name: "NKTelco",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nortex",
        MsoInfo {
            name: "Nortex Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor140",
        MsoInfo {
            name: "North Central Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor030",
        MsoInfo {
            name: "Northland Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor075",
        MsoInfo {
            name: "Northwest Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nor125",
        MsoInfo {
            name: "Norwood Light Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "net010",
        MsoInfo {
            name: "Nsight Telservices",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "dur010",
        MsoInfo {
            name: "Ntec",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nts010",
        MsoInfo {
            name: "NTS Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "new045",
        MsoInfo {
            name: "NU-Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nulink",
        MsoInfo {
            name: "NuLink",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "jam030",
        MsoInfo {
            name: "NVC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "far035",
        MsoInfo {
            name: "OmniTel Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "onesource",
        MsoInfo {
            name: "OneSource Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit230",
        MsoInfo {
            name: "Opelika Power Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "daltonutilities",
        MsoInfo {
            name: "OptiLink",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid140",
        MsoInfo {
            name: "OPTURA",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ote010",
        MsoInfo {
            name: "OTEC Communication Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cci020",
        MsoInfo {
            name: "Packerland Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pan010",
        MsoInfo {
            name: "Panora Telco/Guthrie Center Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "otter",
        MsoInfo {
            name: "Park Region Telephone & Otter Tail Telcom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid050",
        MsoInfo {
            name: "Partner Communications Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "fib010",
        MsoInfo {
            name: "Pathway",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "paulbunyan",
        MsoInfo {
            name: "Paul Bunyan Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pem020",
        MsoInfo {
            name: "Pembroke Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mck010",
        MsoInfo {
            name: "Peoples Rural Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pul010",
        MsoInfo {
            name: "PES Energize",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "phi010",
        MsoInfo {
            name: "Philippi Communications System",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "phonoscope",
        MsoInfo {
            name: "Phonoscope Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pin070",
        MsoInfo {
            name: "Pine Belt Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-pine",
        MsoInfo {
            name: "Pine Bluff Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pin060",
        MsoInfo {
            name: "Pineland Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cam010",
        MsoInfo {
            name: "Pinpoint Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pio060",
        MsoInfo {
            name: "Pioneer Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pioncomm",
        MsoInfo {
            name: "Pioneer Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pioneer",
        MsoInfo {
            name: "Pioneer DTV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pla020",
        MsoInfo {
            name: "Plant TiftNet, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "par010",
        MsoInfo {
            name: "PLWC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pro035",
        MsoInfo {
            name: "PMT",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "vik011",
        MsoInfo {
            name: "Polar Cablevision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pottawatomie",
        MsoInfo {
            name: "Pottawatomie Telephone Co.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "premiercomm",
        MsoInfo {
            name: "Premier Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "psc010",
        MsoInfo {
            name: "PSC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pan020",
        MsoInfo {
            name: "PTCI",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "qco010",
        MsoInfo {
            name: "QCOL",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "qua010",
        MsoInfo {
            name: "Quality Cablevision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rad010",
        MsoInfo {
            name: "Radcliffe Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "car040",
        MsoInfo {
            name: "Rainbow Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rai030",
        MsoInfo {
            name: "Rainier Connect",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ral010",
        MsoInfo {
            name: "Ralls Technologies",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rct010",
        MsoInfo {
            name: "RC Technologies",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "red040",
        MsoInfo {
            name: "Red River Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ree010",
        MsoInfo {
            name: "Reedsburg Utility Commission",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mol010",
        MsoInfo {
            name: "Reliance Connects- Oregon",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "res020",
        MsoInfo {
            name: "Reserve Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-resort",
        MsoInfo {
            name: "Resort TV Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rld010",
        MsoInfo {
            name: "Richland Grant Telephone Cooperative, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "riv030",
        MsoInfo {
            name: "River Valley Telecommunications Coop",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rockportcable",
        MsoInfo {
            name: "Rock Port Cablevision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rsf010",
        MsoInfo {
            name: "RS Fiber",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rtc",
        MsoInfo {
            name: "RTC Communication Corp",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "res040",
        MsoInfo {
            name: "RTC-Reservation Telephone Coop.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "rte010",
        MsoInfo {
            name: "RTEC Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "stc010",
        MsoInfo {
            name: "S&T",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "san020",
        MsoInfo {
            name: "San Bruno Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "san040-01",
        MsoInfo {
            name: "Santel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sav010",
        MsoInfo {
            name: "SCI Broadband-Savage Communications Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sco050",
        MsoInfo {
            name: "Scottsboro Electric Power Board",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "scr010",
        MsoInfo {
            name: "Scranton Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "selco",
        MsoInfo {
            name: "SELCO",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "she010",
        MsoInfo {
            name: "Shentel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "she030",
        MsoInfo {
            name: "Sherwood Mutual Telephone Association, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ind060-ssc",
        MsoInfo {
            name: "Silver Star Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sjoberg",
        MsoInfo {
            name: "Sjoberg's Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sou025",
        MsoInfo {
            name: "SKT",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sky050",
        MsoInfo {
            name: "SkyBest TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcsmi010",
        MsoInfo {
            name: "Smithville Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "woo010",
        MsoInfo {
            name: "Solarus",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sou075",
        MsoInfo {
            name: "South Central Rural Telephone Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sou065",
        MsoInfo {
            name: "South Holt Cablevision, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sou035",
        MsoInfo {
            name: "South Slope Cooperative Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "spa020",
        MsoInfo {
            name: "Spanish Fork Community Network",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "spe010",
        MsoInfo {
            name: "Spencer Municipal Utilities",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "spi005",
        MsoInfo {
            name: "Spillway Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "srt010",
        MsoInfo {
            name: "SRT",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cccsmc010",
        MsoInfo {
            name: "St. Maarten Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sta025",
        MsoInfo {
            name: "Star Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sco020",
        MsoInfo {
            name: "STE",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "uin010",
        MsoInfo {
            name: "STRATA Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sum010",
        MsoInfo {
            name: "Sumner Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "pie010",
        MsoInfo {
            name: "Surry TV/PCSI TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "swa010",
        MsoInfo {
            name: "Swayzee Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sweetwater",
        MsoInfo {
            name: "Sweetwater Cable Television Co",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-talequah",
        MsoInfo {
            name: "Tahlequah Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tct",
        MsoInfo {
            name: "TCT",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tel050",
        MsoInfo {
            name: "Tele-Media Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "com050",
        MsoInfo {
            name: "The Community Agency",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "thr020",
        MsoInfo {
            name: "Three River",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cab140",
        MsoInfo {
            name: "Town & Country Technologies",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tra010",
        MsoInfo {
            name: "Trans-Video",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tre010",
        MsoInfo {
            name: "Trenton TV Cable Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tcc",
        MsoInfo {
            name: "Tri County Communications Cooperative",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tri025",
        MsoInfo {
            name: "TriCounty Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tri110",
        MsoInfo {
            name: "TrioTel Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tro010",
        MsoInfo {
            name: "Troy Cablevision, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tsc",
        MsoInfo {
            name: "TSC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cit220",
        MsoInfo {
            name: "Tullahoma Utilities Board",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tvc030",
        MsoInfo {
            name: "TV Cable of Rensselaer",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tvc015",
        MsoInfo {
            name: "TVC Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cab180",
        MsoInfo {
            name: "TVision",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "twi040",
        MsoInfo {
            name: "Twin Lakes",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "tvtinc",
        MsoInfo {
            name: "Twin Valley",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "uis010",
        MsoInfo {
            name: "Union Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "uni110",
        MsoInfo {
            name: "United Communications - TN",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "uni120",
        MsoInfo {
            name: "United Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "uss020",
        MsoInfo {
            name: "US Sonet",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cab060",
        MsoInfo {
            name: "USA Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "she005",
        MsoInfo {
            name: "USA Communications/Shellsburg, IA",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "val040",
        MsoInfo {
            name: "Valley TeleCom Group",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "val025",
        MsoInfo {
            name: "Valley Telecommunications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "val030",
        MsoInfo {
            name: "Valparaiso Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cla050",
        MsoInfo {
            name: "Vast Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "sul015",
        MsoInfo {
            name: "Venture Communications Cooperative, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ver025",
        MsoInfo {
            name: "Vernon Communications Co-op",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-vicksburg",
        MsoInfo {
            name: "Vicksburg Video",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "vis070",
        MsoInfo {
            name: "Vision Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "volcanotel",
        MsoInfo {
            name: "Volcano Vision, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "vol040-02",
        MsoInfo {
            name: "VolFirst / BLTV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ver070",
        MsoInfo {
            name: "VTel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcvtx010",
        MsoInfo {
            name: "VTX1",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "bci010-02",
        MsoInfo {
            name: "Vyve Broadband",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wab020",
        MsoInfo {
            name: "Wabash Mutual Telephone",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "waitsfield",
        MsoInfo {
            name: "Waitsfield Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wal010",
        MsoInfo {
            name: "Walnut Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wavebroadband",
        MsoInfo {
            name: "Wave",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wav030",
        MsoInfo {
            name: "Waverly Communications Utility",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wbi010",
        MsoInfo {
            name: "WBI",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "web020",
        MsoInfo {
            name: "Webster-Calhoun Cooperative Telephone Association",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wes005",
        MsoInfo {
            name: "West Alabama TV Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "carolinata",
        MsoInfo {
            name: "West Carolina Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wct010",
        MsoInfo {
            name: "West Central Telephone Association",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wes110",
        MsoInfo {
            name: "West River Cooperative Telephone Company",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "ani030",
        MsoInfo {
            name: "WesTel Systems",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "westianet",
        MsoInfo {
            name: "Western Iowa Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "nttcwhi010",
        MsoInfo {
            name: "Whidbey Telecom",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "weh010-white",
        MsoInfo {
            name: "White County Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wes130",
        MsoInfo {
            name: "Wiatel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wik010",
        MsoInfo {
            name: "Wiktel",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wil070",
        MsoInfo {
            name: "Wilkes Communications, Inc./RiverStreet Networks",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wil015",
        MsoInfo {
            name: "Wilson Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "win010",
        MsoInfo {
            name: "Windomnet/SMBS",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "win090",
        MsoInfo {
            name: "Windstream Cable TV",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wcta",
        MsoInfo {
            name: "Winnebago Cooperative Telecom Association",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wtc010",
        MsoInfo {
            name: "WTC",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wil040",
        MsoInfo {
            name: "WTC Communications, Inc.",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "wya010",
        MsoInfo {
            name: "Wyandotte Cable",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "hin020-02",
        MsoInfo {
            name: "X-Stream Services",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "xit010",
        MsoInfo {
            name: "XIT Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "yel010",
        MsoInfo {
            name: "Yelcot Communications",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "mid180-01",
        MsoInfo {
            name: "yondoo",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "cou060",
        MsoInfo {
            name: "Zito Media",
            username_field: None,
            password_field: None,
            login_hostname: None,
        },
    ),
    (
        "slingtv",
        MsoInfo {
            name: "Sling TV",
            username_field: Some("username"),
            password_field: Some("password"),
            login_hostname: Some("identity.sling.com"),
        },
    ),
    (
        "Suddenlink",
        MsoInfo {
            name: "Suddenlink",
            username_field: Some("username"),
            password_field: Some("password"),
            login_hostname: None,
        },
    ),
    (
        "AlticeOne",
        MsoInfo {
            name: "Optimum TV",
            username_field: Some("j_username"),
            password_field: Some("j_password"),
            login_hostname: None,
        },
    ),
];
