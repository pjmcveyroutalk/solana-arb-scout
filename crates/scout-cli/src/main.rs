use scout_core::{
    calculate_economics, ExecutionCosts, FlashCosts, FundingMode, RouteQuote,
};

fn main() {
    println!("ARB Scout V0 — READ ONLY");
    println!("No signing. No wallet. No transaction execution.\n");

    let quote = RouteQuote {
        input_usd: 100.0,
        output_usd: 100.50,
        dex_fees_usd: 0.10,
        price_impact_usd: 0.05,
    };

    let execution = ExecutionCosts {
        base_fee_usd: 0.001,
        priority_fee_usd: 0.01,
        submission_cost_usd: 0.01,
        expected_failure_cost_usd: 0.02,
        safety_reserve_usd: 0.05,
    };

    let flash = FlashCosts {
        borrow_fee_usd: 0.01,
        added_execution_cost_usd: 0.02,
    };

    match calculate_economics(
        &quote,
        &execution,
        FundingMode::Treasury,
        None,
    ) {
        Ok(result) => {
            println!(
                "Treasury expected net: ${:.6}",
                result.expected_net_usd
            );
            println!(
                "Treasury passes minimum: {}",
                result.passes_minimum
            );
        }
        Err(error) => {
            eprintln!("Treasury model error: {error}");
        }
    }

    match calculate_economics(
        &quote,
        &execution,
        FundingMode::Flash,
        Some(&flash),
    ) {
        Ok(result) => {
            println!(
                "Flash expected net: ${:.6}",
                result.expected_net_usd
            );
            println!(
                "Flash passes minimum: {}",
                result.passes_minimum
            );
        }
        Err(error) => {
            eprintln!("Flash model error: {error}");
        }
    }
}
