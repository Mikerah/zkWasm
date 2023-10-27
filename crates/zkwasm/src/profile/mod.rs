use instruction_merge::InstructionMergingProfile;
use instruction_statistic::InstructionStatistic;
use specs::Tables;

mod helper;
mod instruction_merge;
mod instruction_statistic;

pub trait Profiler {
    fn profile_tables(&self);
}

impl Profiler for Tables {
    fn profile_tables(&self) {
        self.execution_table.etable.profile_instruction();

        self.execution_table.etable.estimate_mergeable_instruction();
    }
}
