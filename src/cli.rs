use crate::config;
use crate::storage::Storage;

#[cfg(feature = "openapi")]
use utoipa::OpenApi;

const HELP_TEXT: &str = "\
voltrus — Industrial Modbus monitoring dashboard

USAGE:
    voltrus [CONFIG_PATH]              Start server (default: config.yaml)
    voltrus --help                     Show this help
    voltrus --reset-admin-password [CONFIG]
                                            Reset admin password (prints new password)
    voltrus --dump-openapi [CONFIG]    Print OpenAPI 3.1 spec as JSON and exit
    voltrus backup [OPTIONS]           Create backup archive
    voltrus restore <BACKUP_FILE> [OPTIONS]
                                            Restore from backup archive
    voltrus factory-reset [CONFIG]     Delete all data and reset to defaults
    voltrus db stats [CONFIG]          Print database statistics
    voltrus db vacuum [CONFIG]         Rebuild the database file to return free space to the
                                            filesystem (stop the server first; needs free disk
                                            equal to the database size)
    voltrus edr report [CONFIG]        Show EDR compatibility report
    voltrus edr exclusions [CONFIG]    Show recommended EDR exclusions
    voltrus edr exclusions-ps1 [CONFIG] Generate Defender exclusion PowerShell script
    voltrus edr gpo-guide              Show Group Policy deployment guide
    voltrus hw report                  Show hardware certification report
    voltrus hw benchmark               Run performance benchmarks (default: 10s)
    voltrus hw edge-deploy             Show edge deployment guide
    voltrus hw compat-matrix           Show hardware compatibility matrix
    voltrus hw rpi-quickstart          Show Raspberry Pi quick-start guide
    voltrus project export <PATH>      Export the whole configuration as a YAML project
                                            (directory, or one file when PATH ends in .yaml)
    voltrus project import <PATH> [--mode merge|replace] [--dry-run]
                                            Import a YAML project (one transaction)
    voltrus project validate <PATH>    Check a YAML project without touching a database
    voltrus config export|import|validate [PATH]
                                            Older spelling of the project commands
    voltrus discover --range <CIDR>    Scan subnet for Modbus TCP devices
    voltrus egress list [CONFIG]         List scheduled data egress jobs
    voltrus egress run <ID> [CONFIG]     Manually trigger a data egress job
    voltrus export [OPTIONS]            Export project to .voltrus-scada archive
    voltrus import <FILE> [OPTIONS]     Import project from .voltrus-scada archive
    voltrus template builtins <DIR>     Write the built-in device templates as files into DIR
    voltrus template catalog <DIR>      Write DIR/index.json listing the template files in DIR with sha256
    voltrus station init [--dir DIR] [--port PORT]
                                            Set up this computer as a local station: config.yaml
                                            (127.0.0.1, free port from 3000), data.db, logs/ and the
                                            console sign-in station.toml in DIR (default: the
                                            station directory). Safe to run again.
    voltrus service install|uninstall|start|stop|status [--dir DIR]
                                            Manage the local station server as a background service
                                            (Windows service VoltrusSCADA, macOS LaunchAgent
                                            id.voltrus.scada.server). status exits 0 running,
                                            3 stopped, 4 not installed
    voltrus service run [--dir DIR]     Run the station server (what the service manager starts);
                                            logs to DIR/logs/voltrus.log

STATION DIRECTORY:
    VOLTRUS_STATION_DIR if set, else
    Windows: %ProgramData%\\Voltrus SCADA
    macOS:   ~/Library/Application Support/Voltrus SCADA
    Linux:   $XDG_DATA_HOME/voltrus-scada or ~/.local/share/voltrus-scada

BACKUP OPTIONS:
    --output <PATH>    Output archive path (default: voltrus-backup-YYYYMMDD-HHMMSS.tar.gz)
    --config <PATH>    Config file path for db_path lookup (default: config.yaml)

RESTORE OPTIONS:
    <BACKUP_FILE>      Path to the .tar.gz backup archive
    --config <PATH>    Target config file path (default: config.yaml)
    --force            Skip confirmation prompt

DISCOVER OPTIONS:
    --range <CIDR>     Subnet to scan (e.g., 192.168.1.0/24)
    --port <PORT>      Modbus TCP port (default: 502)
    --config <PATH>    Config file path (default: config.yaml)

EXPORT OPTIONS:
    --output <PATH>    Output archive path (default: project-export.voltrus-scada)
    --config <PATH>    Config file path for db_path lookup (default: config.yaml)

IMPORT OPTIONS:
    <FILE>             Path to the .voltrus-scada archive to import
    --config <PATH>    Config file path for db_path lookup (default: config.yaml)
    --dry-run          Validate archive without writing to database

OPTIONS:
    --help                        Show this help message
    --reset-admin-password        Generate a new random password for admin user

EXAMPLES:
    voltrus                              # Start with config.yaml
    voltrus /etc/voltrus/prod.yaml       # Start with custom config
    voltrus --reset-admin-password       # Reset lost admin password
    voltrus station init                 # Set up this computer as a local station
    voltrus service install && voltrus service start   # ... and run it in the background
    voltrus backup                       # Create backup in current directory
    voltrus backup --output /tmp/bak.tar.gz  # Custom output path
    voltrus restore backup.tar.gz        # Restore from archive
    voltrus restore backup.tar.gz --force  # Restore without confirmation
    voltrus restore backup.tar.gz --config /etc/voltrus/prod.yaml
    voltrus project export ./site        # Export configuration to a directory
    voltrus project export site.yaml     # ... or to a single file
    voltrus project import ./site --dry-run   # Print what an import would change
    voltrus project import site.yaml --mode replace
    voltrus project validate ./site      # Validate project files
    voltrus discover --range 192.168.1.0/24 --port 502
    voltrus export                           # Export project to archive
    voltrus export --output /tmp/project.voltrus-scada
    voltrus import project.voltrus-scada     # Import project from archive
    voltrus import project.voltrus-scada --dry-run  # Validate without importing
";

/// Handle CLI subcommands. Returns true if a subcommand was handled (should exit).
pub async fn handle_cli(args: &[String], ext: &dyn crate::edition::Extension) -> bool {
    if args.len() <= 1 {
        return false;
    }

    // Subcommands owned by an add-on edition (licensing and so on).
    if ext.handle_cli(args).await {
        return true;
    }

    match args[1].as_str() {
        "--help" | "-h" => {
            print!("{}", HELP_TEXT);
            print!("{}", ext.cli_help());
            std::process::exit(0);
        }
        "--reset-admin-password" => handle_reset_admin_password(args).await,
        "config" => handle_project(args, true).await,
        "project" => handle_project(args, false).await,
        "edr" => handle_edr(args),
        "hw" => handle_hw(args),
        "discover" => handle_discover(args).await,
        "backup" => handle_backup(args),
        "restore" => handle_restore(args),
        "factory-reset" => handle_factory_reset(args),
        "db" => handle_db(args),
        "egress" => handle_egress(args).await,
        "export" => handle_export(args),
        "import" => handle_import(args),
        "template" => handle_template(args),
        "station" => crate::station::handle_cli(args).await,
        "service" => crate::service::handle_cli(args),
        "--dump-openapi" => handle_dump_openapi(),
        other if other.starts_with('-') => {
            eprintln!("Unknown option: {}", other);
            eprintln!("Run '{} --help' for usage.", args[0]);
            std::process::exit(1);
        }
        _ => false,
    }
}

/// `voltrus template builtins <DIR>` and `voltrus template catalog <DIR>`: build the files of a public template catalog.
fn handle_template(args: &[String]) -> bool {
    let usage = || {
        eprintln!("Usage: voltrus template builtins <DIR>   Write the built-in templates to DIR");
        eprintln!(
            "       voltrus template catalog <DIR>    Write DIR/index.json (sha256 of every template file in DIR)"
        );
        std::process::exit(1);
    };
    let (Some(sub), Some(dir)) = (args.get(2), args.get(3)) else {
        usage();
        return true;
    };
    let dir = std::path::Path::new(dir);
    let result = match sub.as_str() {
        "builtins" => crate::device_templates::write_builtins(dir)
            .map(|n| format!("Wrote {n} built-in template files to {}", dir.display())),
        "catalog" => crate::device_templates::write_catalog_index(dir).map(|n| {
            format!(
                "Wrote {} listing {n} templates",
                dir.join("index.json").display()
            )
        }),
        _ => {
            usage();
            return true;
        }
    };
    match result {
        Ok(msg) => {
            println!("{msg}");
            true
        }
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
    }
}

async fn handle_reset_admin_password(args: &[String]) -> bool {
    let config_path = args.get(2).map(|s| s.as_str()).unwrap_or("config.yaml");
    let app_config = config::load_config(config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let db_path = app_config.db_path.clone();
    let storage = Storage::new_with_key(&db_path, app_config.db_encryption_key.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("DB error: {e}");
            std::process::exit(1);
        });
    match storage.get_user_by_username("admin").await {
        Ok(None) => {
            eprintln!("Admin user not found. Start the server first to auto-seed the database.");
            std::process::exit(1);
        }
        Ok(Some(user)) => {
            use rand::Rng;
            let password: String = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(16)
                .map(char::from)
                .collect();
            let hash = crate::api::auth::hash_password(&password);
            if let Err(e) = storage.update_user_password(user.id, &hash).await {
                if e.to_string().contains("readonly") || e.to_string().contains("read-only") {
                    eprintln!(
                        "Database is read-only: {}\n\nFix: run with sudo, or change ownership:\n  sudo chown $USER:{}",
                        db_path, db_path
                    );
                    std::process::exit(1);
                }
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            if let Err(e) = storage.set_must_change_password("admin", true).await {
                if e.to_string().contains("readonly") || e.to_string().contains("read-only") {
                    eprintln!(
                        "Database is read-only: {}\n\nFix: run with sudo, or change ownership:\n  sudo chown $USER:{}",
                        db_path, db_path
                    );
                    std::process::exit(1);
                }
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            println!("Admin password reset successfully.");
            println!("New password: {}", password);
            println!("You will be required to change this on next login.");
            true
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_hw(args: &[String]) -> bool {
    let subcommand = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match subcommand {
        "report" => handle_hw_report(args),
        "benchmark" => handle_hw_benchmark(args),
        "edge-deploy" => handle_hw_edge_deploy(args),
        "compat-matrix" => handle_hw_compat_matrix(),
        "rpi-quickstart" => {
            let guide = crate::hardware_cert::edge_deploy::generate_rpi_quickstart();
            println!("{}", guide);
            true
        }
        _ => {
            eprintln!(
                "Usage: {} hw [report|benchmark|edge-deploy|compat-matrix|rpi-quickstart]",
                args[0]
            );
            eprintln!();
            eprintln!("Commands:");
            eprintln!("  report          Show hardware certification report");
            eprintln!("  benchmark       Run performance benchmarks (default: 10s)");
            eprintln!("  edge-deploy     Show edge deployment guide for this platform");
            eprintln!("  compat-matrix   Show hardware compatibility matrix");
            eprintln!("  rpi-quickstart  Show Raspberry Pi quick-start guide");
            std::process::exit(1);
        }
    }
}

fn handle_hw_report(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    let platform = crate::hardware_cert::platform::detect_platform(&db_path);
    let compatibility = crate::hardware_cert::compat_matrix::assess_compatibility(&platform);
    let storage = crate::hardware_cert::compat_matrix::assess_storage(&platform);

    println!("=== Voltrus Hardware Certification Report ===");
    println!();
    println!("Platform:");
    println!("  Hostname:    {}", platform.hostname);
    println!("  OS:          {}", platform.os);
    println!("  Arch:        {}", platform.arch);
    println!(
        "  CPU:         {} ({} cores)",
        platform.cpu_model, platform.cpu_cores
    );
    println!("  RAM:         {} MB", platform.ram_total_mb);
    println!(
        "  Disk:        {:.1} / {:.1} GB ({:.0}% used)",
        platform.disk_total_gb - platform.disk_available_gb,
        platform.disk_total_gb,
        if platform.disk_total_gb > 0.0 {
            ((platform.disk_total_gb - platform.disk_available_gb) / platform.disk_total_gb) * 100.0
        } else {
            0.0
        }
    );
    println!("  Board:       {}", platform.board_type);
    println!("  Storage:     {}", platform.storage_type);
    println!();

    println!("Tier Compatibility:");
    let tiers = [
        ("Engineering", &compatibility.engineering),
        ("Starter", &compatibility.starter),
        ("Professional", &compatibility.professional),
        ("Enterprise", &compatibility.enterprise),
    ];
    for (name, assessment) in &tiers {
        let status = if assessment.compatible {
            "✓ COMPATIBLE"
        } else {
            "✗ NOT COMPATIBLE"
        };
        println!("  {} — {}", name, status);
        if assessment.compatible {
            println!(
                "    Max tags: {} | Max poll rate: {} Hz | Compression: {}",
                assessment.max_tags,
                assessment.max_poll_rate,
                if assessment.compression_recommended {
                    "recommended"
                } else {
                    "optional"
                }
            );
        }
        for limitation in &assessment.limitations {
            println!("    ⚠ {}", limitation);
        }
    }

    println!();
    println!("Storage Assessment:");
    println!("  Type:                {}", storage.storage_type);
    println!(
        "  Write optimization:  {}",
        if storage.write_optimization_critical {
            "CRITICAL"
        } else {
            "recommended"
        }
    );
    println!(
        "  Est. endurance:      {:.1} years (at 10K tags @ 1Hz)",
        storage.estimated_endurance_years
    );
    println!(
        "  Batch writes:        {}",
        if storage.batch_writes_active {
            "active"
        } else {
            "disabled"
        }
    );

    if !storage.storage_recommendations.is_empty() {
        println!("  Recommendations:");
        for rec in &storage.storage_recommendations {
            println!("    → {}", rec);
        }
    }

    println!();
    println!("Run 'voltrus hw benchmark' to measure actual performance.");
    println!("Run 'voltrus hw edge-deploy' for platform-specific deployment guide.");

    true
}

fn handle_hw_benchmark(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    // Parse optional duration
    let duration = args
        .get(3)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10)
        .min(60);

    println!("=== Voltrus Hardware Benchmark ===");
    println!(
        "Duration: {}s per test (4 tests, ~{}s total)",
        duration,
        duration * 4
    );
    println!("Running benchmarks...");

    let platform = crate::hardware_cert::platform::detect_platform(&db_path);

    match crate::hardware_cert::benchmark::run_benchmarks(&platform, Some(duration)) {
        Ok(results) => {
            println!();
            println!("Benchmark Results:");
            println!(
                "  Platform: {} ({})",
                results.platform.board_type, results.platform.arch
            );
            println!(
                "  RAM: {} MB | Cores: {}",
                results.platform.ram_total_mb, results.platform.cpu_cores
            );
            println!();

            for detail in &results.details {
                let status = if detail.passed { "PASS" } else { "FAIL" };
                let unit = match detail.name.as_str() {
                    "tag_write" => "writes/s",
                    "batch_insert" => "rows/s",
                    "range_query" => "queries/s",
                    "compression" => "rows/s",
                    _ => "ops/s",
                };
                println!(
                    "  [{}] {:20} {:>12.0} {} ({} ms)",
                    status, detail.name, detail.throughput, unit, detail.duration_ms
                );
            }

            println!();
            println!("Recommendations:");
            println!(
                "  Max tags:       {} (at 1 Hz)",
                results.recommended_max_tags
            );
            println!("  Max poll rate:  {} Hz", results.recommended_poll_rate);
            println!("  Write amplification: {:.1}x", results.write_amplification);
            println!("  Peak memory:    {:.0} MB", results.peak_memory_mb);

            let all_passed = results.details.iter().all(|d| d.passed);
            println!();
            if all_passed {
                println!("✓ All benchmarks PASSED — this hardware meets minimum requirements.");
            } else {
                println!("✗ Some benchmarks FAILED — hardware may not meet minimum requirements.");
                println!("  Consider reducing tag count or poll rate for this hardware.");
            }
        }
        Err(e) => {
            eprintln!("Benchmark failed: {e}");
            std::process::exit(1);
        }
    }

    true
}

fn handle_hw_edge_deploy(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    let platform = crate::hardware_cert::platform::detect_platform(&db_path);
    let guide = crate::hardware_cert::edge_deploy::generate_edge_guide(&platform);
    println!("{}", guide);
    true
}

fn handle_hw_compat_matrix() -> bool {
    let matrix = crate::hardware_cert::compat_matrix::get_compatibility_matrix();

    println!("=== Voltrus Hardware Compatibility Matrix ===");
    println!();

    for entry in &matrix {
        println!("Platform: {}", entry.platform);
        println!("  CPU:      {}", entry.cpu);
        println!("  RAM:      {} MB", entry.ram_mb);
        println!("  Storage:  {}", entry.storage);
        println!("  Tiers:    {}", entry.supported_tiers.join(", "));
        println!(
            "  Max tags: {} | Poll rate: {} Hz",
            entry.max_tags, entry.max_poll_rate
        );
        println!("  Notes:    {}", entry.notes);
        println!();
    }

    println!("Disclaimer: Recommendations based on testing with typical SCADA workloads.");
    println!("Actual performance depends on tag count, polling rate, and enabled features.");

    true
}

fn handle_edr(args: &[String]) -> bool {
    let subcommand = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match subcommand {
        "report" => handle_edr_report(args),
        "exclusions" => handle_edr_exclusions(args),
        "exclusions-ps1" => handle_edr_exclusions_ps1(args),
        "gpo-guide" => {
            let guide = crate::edr_compat::generate_gpo_guidance();
            println!("{}", guide);
            true
        }
        _ => {
            eprintln!(
                "Usage: {} edr [report|exclusions|exclusions-ps1|gpo-guide] [CONFIG_PATH]",
                args[0]
            );
            eprintln!();
            eprintln!("Commands:");
            eprintln!("  report          Show full EDR compatibility report");
            eprintln!("  exclusions      Show recommended exclusion paths");
            eprintln!("  exclusions-ps1  Generate Windows Defender PowerShell exclusion script");
            eprintln!("  gpo-guide       Show Group Policy deployment guide");
            std::process::exit(1);
        }
    }
}

fn handle_edr_report(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    let detected = crate::edr_compat::detect_edr_products();
    let exclusions = crate::edr_compat::generate_exclusions(&db_path);

    println!("=== Voltrus EDR Compatibility Report ===");
    println!();

    if detected.is_empty() {
        println!("No EDR/antivirus products detected.");
    } else {
        println!("Detected EDR/antivirus products:");
        for product in &detected {
            println!(
                "  [{}] {} ({})",
                product.category, product.name, product.vendor
            );
            if let Some(ref notes) = product.notes {
                println!("    Notes: {}", notes);
            }
        }
    }

    println!();
    println!("Recommended exclusions:");
    for exc in &exclusions {
        let priority_icon = match exc.priority.as_str() {
            "critical" => "[CRITICAL]",
            "recommended" => "[REC]     ",
            _ => "[OPT]     ",
        };
        println!("  {} {:10} {}", priority_icon, exc.exclusion_type, exc.path);
        println!("             Reason: {}", exc.reason);
    }

    println!();
    println!("Run 'voltrus edr exclusions-ps1' to generate a PowerShell script");
    println!("Run 'voltrus edr gpo-guide' for Group Policy deployment instructions");

    true
}

fn handle_edr_exclusions(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    let exclusions = crate::edr_compat::generate_exclusions(&db_path);
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "exclusions": exclusions,
        "count": exclusions.len(),
    }))
    .unwrap_or_default();
    println!("{}", json);
    true
}

fn handle_edr_exclusions_ps1(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "data.db".to_string());

    let exclusions = crate::edr_compat::generate_exclusions(&db_path);
    let script = crate::edr_compat::generate_defender_ps1(&exclusions);
    println!("{}", script);
    true
}

/// `voltrus project export|import|validate`, and the older
/// `voltrus config export|import|validate` spelling of the same commands.
async fn handle_project(args: &[String], legacy_config: bool) -> bool {
    let cmd = if legacy_config { "config" } else { "project" };
    let usage = |code: i32| -> ! {
        eprintln!(
            "Usage: {0} {cmd} export <PATH> [--config <PATH>]\n       \
             {0} {cmd} import <PATH> [--mode merge|replace] [--dry-run] [--config <PATH>]\n       \
             {0} {cmd} validate <PATH>\n\n\
             PATH is a directory (one file per entity) or a single .yaml file.\n\
             Secrets are written as ${{ENV_NAME}} references and resolved from the\n\
             environment on import. See docs/project-format.md.",
            args[0]
        );
        std::process::exit(code);
    };
    let sub = args.get(2).map(String::as_str).unwrap_or("");
    if !matches!(sub, "export" | "import" | "validate") {
        usage(1);
    }

    let mut path: Option<String> = None;
    let mut config_path = "config.yaml".to_string();
    let mut mode = crate::project_yaml::Mode::Merge;
    let mut dry_run = false;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--config" if i + 1 < args.len() => {
                config_path = args[i + 1].clone();
                i += 2;
            }
            "--mode" if i + 1 < args.len() => {
                mode = args[i + 1].parse().unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                });
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--help" | "-h" => usage(0),
            flag if flag.starts_with('-') => {
                eprintln!("Unknown {cmd} option: {flag}");
                usage(1);
            }
            arg if path.is_none() => {
                path = Some(arg.to_string());
                i += 1;
            }
            // `config export DIR CONFIG` took the config file positionally.
            arg if legacy_config => {
                config_path = arg.to_string();
                i += 1;
            }
            arg => {
                eprintln!("Unexpected argument: {arg}");
                usage(1);
            }
        }
    }
    let path = match path {
        Some(p) => p,
        None if legacy_config => "config".to_string(),
        None => usage(1),
    };
    let target = std::path::Path::new(&path);

    if sub == "validate" {
        let doc = match crate::project_yaml::read_project(target) {
            Ok(d) => d,
            Err(e) => {
                print_project_error(&e);
                std::process::exit(1);
            }
        };
        for w in &doc.warnings {
            println!("WARN: {w}");
        }
        let issues = crate::project_yaml::validate_doc(&doc, None, true);
        for issue in &issues {
            println!("{issue}");
        }
        let errors = issues
            .iter()
            .filter(|i| i.severity == crate::project_yaml::Severity::Error)
            .count();
        let entities: usize = doc.sections.values().map(Vec::len).sum();
        if errors == 0 {
            println!(
                "OK: {path} is a valid {} project ({entities} entities)",
                crate::project_yaml::FORMAT
            );
        } else {
            println!("{errors} error(s) in {path}");
        }
        std::process::exit(if errors == 0 { 0 } else { 1 });
    }

    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let storage =
        Storage::new_with_key(&app_config.db_path, app_config.db_encryption_key.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("DB error: {e}");
                std::process::exit(1);
            });

    if sub == "export" {
        let doc = match crate::project_yaml::export_project_yaml(&storage).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Export failed: {e:#}");
                std::process::exit(1);
            }
        };
        if let Err(e) = crate::project_yaml::write_project(&doc, target) {
            eprintln!("Export failed: {e:#}");
            std::process::exit(1);
        }
        for w in &doc.warnings {
            eprintln!("warning: {w}");
        }
        let entities: usize = doc.sections.values().map(Vec::len).sum();
        println!("Project exported to {path} ({entities} entities)");
        return true;
    }

    let doc = match crate::project_yaml::read_project(target) {
        Ok(d) => d,
        Err(e) => {
            print_project_error(&e);
            std::process::exit(1);
        }
    };
    let result = if dry_run {
        crate::project_yaml::plan_import(&storage, &doc, mode).await
    } else {
        crate::project_yaml::apply_import(&storage, &doc, mode).await
    };
    match result {
        Ok(plan) => {
            print!("{}", plan.render());
            if dry_run {
                println!("Dry run: nothing was written.");
            } else {
                println!("Project imported from {path}");
            }
            true
        }
        Err(e) => {
            print_project_error(&e);
            eprintln!("Nothing was written.");
            std::process::exit(1);
        }
    }
}

fn print_project_error(e: &anyhow::Error) {
    match e
        .chain()
        .find_map(|c| c.downcast_ref::<crate::project_yaml::InvalidProject>())
    {
        Some(invalid) => {
            for issue in &invalid.0 {
                eprintln!("{issue}");
            }
        }
        None => eprintln!("Error: {e:#}"),
    }
}

async fn handle_discover(args: &[String]) -> bool {
    let mut range = String::new();
    let mut port: u16 = 502;
    let mut config_path = "config.yaml".to_string();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--range" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --range requires a CIDR argument (e.g., 192.168.1.0/24)");
                    std::process::exit(1);
                }
                range = args[i + 1].clone();
                i += 2;
            }
            "--port" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --port requires a port number");
                    std::process::exit(1);
                }
                port = args[i + 1].parse().unwrap_or(502);
                i += 2;
            }
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
                config_path = args[i + 1].clone();
                i += 2;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown discover option: {}", flag);
                eprintln!("Usage: {} discover --range <CIDR> [--port PORT]", args[0]);
                std::process::exit(1);
            }
            _ => {
                eprintln!("Unexpected argument: {}", args[i]);
                eprintln!("Usage: {} discover --range <CIDR> [--port PORT]", args[0]);
                std::process::exit(1);
            }
        }
    }
    if range.is_empty() {
        eprintln!("Usage: {} discover --range <CIDR> [--port PORT]", args[0]);
        std::process::exit(1);
    }
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let _storage =
        Storage::new_with_key(&app_config.db_path, app_config.db_encryption_key.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("DB error: {e}");
                std::process::exit(1);
            });
    eprintln!("Scanning {} port {} for Modbus TCP devices...", range, port);
    match crate::discovery::scan_network(&range, port, 500).await {
        Ok(devices) => {
            let json = serde_json::to_string_pretty(&devices).unwrap_or_default();
            println!("{}", json);
            if devices.is_empty() {
                eprintln!("No Modbus devices found on {} port {}", range, port);
            } else {
                eprintln!("Found {} device(s)", devices.len());
            }
            true
        }
        Err(e) => {
            eprintln!("Scan error: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_backup(args: &[String]) -> bool {
    let mut config_path = "config.yaml".to_string();
    let mut output_path: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
                config_path = args[i + 1].clone();
                i += 2;
            }
            "--output" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --output requires a path argument");
                    std::process::exit(1);
                }
                output_path = Some(args[i + 1].clone());
                i += 2;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown backup option: {}", flag);
                eprintln!(
                    "Usage: {} backup [--config <PATH>] [--output <PATH>]",
                    args[0]
                );
                std::process::exit(1);
            }
            _ => {
                eprintln!("Unexpected argument: {}", args[i]);
                eprintln!(
                    "Usage: {} backup [--config <PATH>] [--output <PATH>]",
                    args[0]
                );
                std::process::exit(1);
            }
        }
    }
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let opts = crate::backup::BackupOptions {
        config_path,
        db_path: app_config.db_path.clone(),
        output_path,
    };
    match crate::backup::run_backup(opts) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("Backup failed: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_restore(args: &[String]) -> bool {
    let mut config_path = "config.yaml".to_string();
    let mut archive_path: Option<String> = None;
    let mut force = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
                config_path = args[i + 1].clone();
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown restore option: {}", flag);
                eprintln!(
                    "Usage: {} restore <BACKUP_FILE> [--config <PATH>] [--force]",
                    args[0]
                );
                std::process::exit(1);
            }
            arg => {
                if archive_path.is_some() {
                    eprintln!("Unexpected argument: {}", arg);
                    eprintln!(
                        "Usage: {} restore <BACKUP_FILE> [--config <PATH>] [--force]",
                        args[0]
                    );
                    std::process::exit(1);
                }
                archive_path = Some(arg.to_string());
                i += 1;
            }
        }
    }
    let archive_path = match archive_path {
        Some(p) => p,
        None => {
            eprintln!(
                "Usage: {} restore <BACKUP_FILE> [--config <PATH>] [--force]",
                args[0]
            );
            std::process::exit(1);
        }
    };
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let opts = crate::backup::RestoreOptions {
        archive_path,
        config_path,
        db_path: app_config.db_path.clone(),
        force,
    };
    match crate::backup::run_restore(opts) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("Restore failed: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_factory_reset(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "voltrus.db".to_string());

    println!(
        "WARNING: This will permanently delete all data in {}",
        db_path
    );
    println!("Type 'yes' to confirm:");

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() || input.trim() != "yes" {
        println!("Aborted.");
        return true;
    }

    match std::fs::remove_file(&db_path) {
        Ok(()) => {
            let _ = std::fs::remove_file(format!("{}-wal", db_path));
            let _ = std::fs::remove_file(format!("{}-shm", db_path));
            println!("Database deleted. Voltrus will create a fresh database on next start.");
            true
        }
        Err(e) => {
            eprintln!("Error deleting database: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_db(args: &[String]) -> bool {
    if args.len() < 3 {
        eprintln!("Usage: {} db <stats|vacuum> [CONFIG_PATH]", args[0]);
        return true;
    }

    match args[2].as_str() {
        "stats" => handle_db_stats(args),
        "vacuum" => handle_db_vacuum(args),
        other => {
            eprintln!("Unknown db subcommand: {}", other);
            true
        }
    }
}

fn handle_db_stats(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "voltrus.db".to_string());

    let storage = match Storage::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open database: {e}");
            std::process::exit(1);
        }
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match storage.get_compression_stats().await {
            Ok(stats) => {
                println!("=== Voltrus Database Statistics ===");
                println!("Database: {}", db_path);
                println!("Size: {:.2} MB", stats.db_size_bytes as f64 / 1_048_576.0);
                println!();
                println!("Raw (unpacked rows):   {} rows", stats.hot_rows);
                println!(
                    "Packed (full res):     {} samples in {} blocks, {:.2} MB ({:.2} bytes/sample)",
                    stats.packed.samples,
                    stats.packed.blocks,
                    stats.packed.payload_bytes as f64 / 1_048_576.0,
                    if stats.packed.samples > 0 {
                        stats.packed.payload_bytes as f64 / stats.packed.samples as f64
                    } else {
                        0.0
                    }
                );
                println!("Hourly aggregates:     {} rows", stats.warm_rows);
                println!("Daily aggregates:      {} rows", stats.cold_rows);
                let free_pages = storage.query_pragma("freelist_count").await.unwrap_or_default();
                match storage.query_pragma("auto_vacuum").await.as_deref() {
                    Ok("2") => println!("Free pages:            {free_pages} (auto_vacuum=incremental: returned by the retention worker)"),
                    _ => println!(
                        "Free pages:            {free_pages} (auto_vacuum off: the file does not shrink; run `voltrus db vacuum` with the server stopped to reclaim it)"
                    ),
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    });
    true
}

/// `voltrus db vacuum`: rebuild the database file with `VACUUM`, which returns
/// every free page to the filesystem and switches the file to
/// `auto_vacuum=INCREMENTAL` so the server can keep it small from then on.
fn handle_db_vacuum(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "voltrus.db".to_string());

    if !std::path::Path::new(&db_path).exists() {
        eprintln!("Database not found: {db_path}");
        std::process::exit(1);
    }
    let storage = match Storage::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open database: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "Rebuilding {db_path} (stop the server first; this needs free disk equal to the database size)..."
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match storage.vacuum_full().await {
            Ok(report) => {
                println!(
                    "Done: {:.2} MB -> {:.2} MB (database plus WAL).",
                    report.before_bytes as f64 / 1_048_576.0,
                    report.after_bytes as f64 / 1_048_576.0
                );
            }
            Err(e) => {
                eprintln!("Vacuum failed: {e:#}");
                std::process::exit(1);
            }
        }
    });
    true
}

fn handle_dump_openapi() -> bool {
    #[cfg(feature = "openapi")]
    {
        let spec = crate::api::openapi::ApiDoc::openapi();
        match serde_json::to_string_pretty(&spec) {
            Ok(json) => {
                println!("{}", json);
                true
            }
            Err(e) => {
                eprintln!("Error serializing OpenAPI spec: {e}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(feature = "openapi"))]
    {
        eprintln!("Error: --dump-openapi requires the 'openapi' feature.");
        eprintln!("Build with: cargo build --features openapi");
        std::process::exit(1);
    }
}

fn find_config_path(args: &[String]) -> String {
    for arg in args.iter().skip(2) {
        if (!arg.starts_with('-') && !arg.contains('/')
            || arg.ends_with(".yaml")
            || arg.ends_with(".yml"))
            && arg != "factory-reset"
            && arg != "db"
            && arg != "stats"
            && arg != "vacuum"
            && arg != "edr"
            && arg != "hw"
            && arg != "egress"
            && arg != "list"
            && arg != "run"
        {
            return arg.clone();
        }
    }
    "config.yaml".to_string()
}

async fn handle_egress(args: &[String]) -> bool {
    if args.len() < 3 {
        eprintln!("Usage: {} egress <list|run> [OPTIONS]", args[0]);
        return true;
    }

    match args[2].as_str() {
        "list" => handle_egress_list(args),
        "run" => handle_egress_run(args).await,
        other => {
            eprintln!("Unknown egress subcommand: {}", other);
            true
        }
    }
}

fn handle_egress_list(args: &[String]) -> bool {
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "voltrus.db".to_string());
    let storage = match Storage::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open database: {e}");
            std::process::exit(1);
        }
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match storage.list_egress_jobs().await {
            Ok(jobs) => {
                if jobs.is_empty() {
                    println!("No egress jobs configured.");
                } else {
                    println!("=== Data Egress Jobs ===");
                    for job in &jobs {
                        let status = job.last_status.as_deref().unwrap_or("never run");
                        let last = job
                            .last_run
                            .map(|t| {
                                time::OffsetDateTime::from_unix_timestamp(t)
                                    .map(|dt| {
                                        dt.format(&time::format_description::well_known::Rfc3339)
                                            .unwrap_or_else(|_| t.to_string())
                                    })
                                    .unwrap_or_else(|_| t.to_string())
                            })
                            .unwrap_or_else(|| "never".to_string());
                        println!(
                            "  [{}] {} (id={}) — {} {} → {} ({}) last: {} {}",
                            if job.enabled { "ON" } else { "OFF" },
                            job.name,
                            job.id,
                            job.schedule,
                            job.time_range,
                            job.destination,
                            job.format,
                            last,
                            if let Some(ref err) = job.last_error {
                                format!("error: {}", err)
                            } else {
                                status.to_string()
                            }
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    });
    true
}

async fn handle_egress_run(args: &[String]) -> bool {
    if args.len() < 4 {
        eprintln!("Usage: {} egress run <JOB_ID> [CONFIG_PATH]", args[0]);
        return true;
    }
    let job_id: i64 = match args[3].parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("Invalid job ID: {}", args[3]);
            return true;
        }
    };
    let db_path = config::load_config(&find_config_path(args))
        .map(|c| c.db_path.clone())
        .unwrap_or_else(|_| "voltrus.db".to_string());
    let storage = match Storage::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open database: {e}");
            std::process::exit(1);
        }
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match storage.get_egress_job(job_id).await {
            Ok(Some(job)) => {
                println!("Running egress job '{}' (id={})...", job.name, job.id);
                match crate::integration::egress::execute_egress_job(&storage, &job).await {
                    Ok(()) => println!("Egress job completed successfully."),
                    Err(e) => {
                        eprintln!("Egress job failed: {e}");
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => {
                eprintln!("Egress job {} not found.", job_id);
            }
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    });
    true
}

fn handle_export(args: &[String]) -> bool {
    let mut config_path = "config.yaml".to_string();
    let mut output_path = "project-export.voltrus-scada".to_string();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
                config_path = args[i + 1].clone();
                i += 2;
            }
            "--output" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --output requires a path argument");
                    std::process::exit(1);
                }
                output_path = args[i + 1].clone();
                i += 2;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown export option: {}", flag);
                eprintln!(
                    "Usage: {} export [--config <PATH>] [--output <PATH>]",
                    args[0]
                );
                std::process::exit(1);
            }
            _ => {
                eprintln!("Unexpected argument: {}", args[i]);
                eprintln!(
                    "Usage: {} export [--config <PATH>] [--output <PATH>]",
                    args[0]
                );
                std::process::exit(1);
            }
        }
    }
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let _storage =
        Storage::new_with_key(&app_config.db_path, app_config.db_encryption_key.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("DB error: {e}");
                std::process::exit(1);
            });
    let _rt = tokio::runtime::Runtime::new().unwrap();
    match crate::project_export::export_project(&app_config.db_path, &output_path) {
        Ok(manifest) => {
            println!("Project exported to: {}", output_path);
            println!("  Devices: {}", manifest.device_count);
            println!("  Tags:    {}", manifest.tag_count);
            println!("  Screens: {}", manifest.screen_count);
            true
        }
        Err(e) => {
            eprintln!("Export failed: {e}");
            std::process::exit(1);
        }
    }
}

fn handle_import(args: &[String]) -> bool {
    let mut config_path = "config.yaml".to_string();
    let mut input_path: Option<String> = None;
    let mut dry_run = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
                config_path = args[i + 1].clone();
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown import option: {}", flag);
                eprintln!(
                    "Usage: {} import <FILE> [--config <PATH>] [--dry-run]",
                    args[0]
                );
                std::process::exit(1);
            }
            arg => {
                if input_path.is_some() {
                    eprintln!("Unexpected argument: {}", arg);
                    eprintln!(
                        "Usage: {} import <FILE> [--config <PATH>] [--dry-run]",
                        args[0]
                    );
                    std::process::exit(1);
                }
                input_path = Some(arg.to_string());
                i += 1;
            }
        }
    }
    let input_path = match input_path {
        Some(p) => p,
        None => {
            eprintln!(
                "Usage: {} import <FILE> [--config <PATH>] [--dry-run]",
                args[0]
            );
            std::process::exit(1);
        }
    };
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        eprintln!("Config error: {e}");
        std::process::exit(1);
    });
    let _storage =
        Storage::new_with_key(&app_config.db_path, app_config.db_encryption_key.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("DB error: {e}");
                std::process::exit(1);
            });
    let _rt = tokio::runtime::Runtime::new().unwrap();
    match crate::project_export::import_project(&app_config.db_path, &input_path, dry_run) {
        Ok(manifest) => {
            if dry_run {
                println!("Dry-run validation passed for: {}", input_path);
            } else {
                println!("Project imported from: {}", input_path);
            }
            println!("  Devices: {}", manifest.device_count);
            println!("  Tags:    {}", manifest.tag_count);
            println!("  Screens: {}", manifest.screen_count);
            if dry_run {
                println!("  (no data was written — use without --dry-run to import)");
            }
            true
        }
        Err(e) => {
            eprintln!("Import failed: {e}");
            std::process::exit(1);
        }
    }
}
